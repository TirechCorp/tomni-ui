//! Escolha do backend dos snapshots: CPU (padrão) ou GPU.
//!
//! As referências em `snapshots/` são geradas na CPU. Com `TOMNI_UI_BACKEND=gpu` (e a feature
//! `gpu` deste crate), os mesmos testes rasterizam na GPU e comparam com elas: é o teste
//! GPU × CPU do M4, com o mesmo limite de 0,5% e uma tolerância por canal
//! ([`GPU_CHANNEL_TOLERANCE`](crate::snapshot::GPU_CHANNEL_TOLERANCE)).

use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CpuCanvas, Pixmap};

/// Variável de ambiente que escolhe o backend: `cpu` (padrão) ou `gpu`.
pub const BACKEND_ENV: &str = "TOMNI_UI_BACKEND";

/// Backend que rasteriza os snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// `vello_cpu` sem SIMD, determinístico. Gera as referências.
    Cpu,
    /// `vello` sobre `wgpu`, no adaptador padrão do sistema.
    Gpu,
}

impl Backend {
    /// Backend pedido em [`BACKEND_ENV`].
    ///
    /// # Panics
    ///
    /// Com um valor desconhecido, ou com `gpu` sem a feature `gpu` deste crate.
    pub fn current() -> Self {
        match std::env::var(BACKEND_ENV).as_deref() {
            Err(_) | Ok("" | "cpu") => Self::Cpu,
            Ok("gpu") if cfg!(feature = "gpu") => Self::Gpu,
            Ok("gpu") => panic!(
                "{BACKEND_ENV}=gpu requer a feature `gpu`: cargo test -p tomni-ui-test --features gpu"
            ),
            Ok(other) => panic!("{BACKEND_ENV}={other:?} desconhecido; use `cpu` ou `gpu`"),
        }
    }
}

impl Backend {
    /// Diferença por canal aceita contra as referências: zero na CPU e
    /// [`GPU_CHANNEL_TOLERANCE`](crate::snapshot::GPU_CHANNEL_TOLERANCE) na GPU.
    pub fn tolerance(self) -> u8 {
        match self {
            Self::Cpu => 0,
            Self::Gpu => crate::snapshot::GPU_CHANNEL_TOLERANCE,
        }
    }
}

/// Desenha uma cena de `width` × `height` no [`Backend::current`] e devolve os pixels.
pub fn render(
    width: u16,
    height: u16,
    background: Color,
    draw: impl FnOnce(&mut dyn Canvas),
) -> Pixmap {
    match Backend::current() {
        Backend::Cpu => {
            let mut canvas = CpuCanvas::deterministic(width, height);
            draw(&mut canvas);
            canvas.render(background)
        }
        #[cfg(feature = "gpu")]
        Backend::Gpu => gpu::render(width, height, background, draw),
        #[cfg(not(feature = "gpu"))]
        Backend::Gpu => unreachable!("Backend::current só devolve Gpu com a feature `gpu`"),
    }
}

#[cfg(feature = "gpu")]
mod gpu {
    use std::sync::{Mutex, OnceLock, PoisonError};

    use tomni_ui_render::peniko::Color;
    use tomni_ui_render::{Canvas, GpuCanvas, GpuRenderer, Pixmap};

    /// Um dispositivo para todos os testes do processo: criar um por teste custa caro.
    fn renderer() -> &'static Mutex<GpuRenderer> {
        static RENDERER: OnceLock<Mutex<GpuRenderer>> = OnceLock::new();
        RENDERER.get_or_init(|| {
            let renderer = GpuRenderer::new().unwrap_or_else(|e| panic!("backend GPU: {e}"));
            let info = renderer.adapter_info();
            eprintln!(
                "tomni-ui-test: GPU {} ({:?}, {})",
                info.name, info.backend, info.driver
            );
            Mutex::new(renderer)
        })
    }

    pub(super) fn render(
        width: u16,
        height: u16,
        background: Color,
        draw: impl FnOnce(&mut dyn Canvas),
    ) -> Pixmap {
        let mut canvas = GpuCanvas::new(width.into(), height.into(), 1.0);
        draw(&mut canvas);
        renderer()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .render(&canvas, background)
            .unwrap_or_else(|e| panic!("backend GPU: {e}"))
    }
}
