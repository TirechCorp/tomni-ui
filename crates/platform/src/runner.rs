//! Loop de eventos do `winit`: janelas, superfícies da GPU e tradução dos eventos.
//!
//! Cada janela do processo é uma [`Janela`], com a sua superfície, o seu `Handler` e o seu
//! estado de ponteiro, cliques e IME. O `RenderContext` e os `Renderer` são compartilhados.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[cfg(target_os = "android")]
use tomni_ui_core::Insets;
use tomni_ui_core::kurbo::{Point, Vec2};
use tomni_ui_core::stages;
use tomni_ui_core::windows::{self, OpenWindow, WindowGeometry, WindowHandle};
use tomni_ui_core::{
    Cursor, Ime, InputPurpose, Modifiers, PointerKind, ResizeEdge, TextInput, WindowChrome,
    WindowRequest,
};
use tomni_ui_render::vello::util::{RenderContext, RenderSurface};
use tomni_ui_render::vello::{Renderer, wgpu};
use tomni_ui_render::{CpuCanvas, GpuCanvas, new_renderer, render_params};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{
    CursorIcon, ImePurpose, ResizeDirection, Theme, UserAttentionType, Window, WindowAttributes,
    WindowId,
};

use crate::frames::{FrameTrace, Pacer, Reason, SurfaceFailures};
use crate::render_guard::{Mode, RenderGuard};
use crate::{Backdrop, Blink, ClickCounter, Error, Handler, WindowOptions, keys};

/// Pixels rolados por um dente da roda do mouse: três linhas de 16 px, o padrão do Windows.
const WHEEL_LINE: f64 = 48.0;

/// Abre a janela e roda `handler` até ela ser fechada.
///
/// Bloqueia a thread, que precisa ser a principal (exigência do macOS e do iOS). No Android,
/// chamar de `android_main` com `WindowOptions::android_app`. Os pedidos de
/// [`open_window`](tomni_ui_core::open_window) são recusados; use [`run_windows`] para
/// atendê-los.
pub fn run<H: Handler>(options: WindowOptions, handler: H) -> Result<(), Error> {
    run_windows(options, handler, |_| None)
}

/// Como [`run`], e atende os pedidos de [`open_window`](tomni_ui_core::open_window): para cada
/// janela pedida, `factory` monta o `Handler` dela (com a tela do pedido) ou devolve `None`
/// para recusar. O app termina quando a janela principal, ou a última, fecha.
pub fn run_windows<H: Handler>(
    options: WindowOptions,
    handler: H,
    factory: impl FnMut(OpenWindow) -> Option<H> + 'static,
) -> Result<(), Error> {
    #[cfg_attr(
        not(any(target_os = "android", target_os = "linux")),
        allow(unused_mut)
    )]
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "android")]
    {
        use winit::platform::android::EventLoopBuilderExtAndroid;
        let app = options.android_app.clone().ok_or(Error::NoAndroidApp)?;
        builder.with_android_app(app);
    }
    // O WebKitGTK só põe a janela filha do `WebView` numa janela X11: numa sessão Wayland, a
    // janela abre pelo XWayland.
    #[cfg(target_os = "linux")]
    if tomni_ui_core::native::x11_required() {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11();
    }
    let event_loop = builder.build().map_err(Error::EventLoop)?;
    // Uma thread que lê dados (o transporte do `Terminal`) acorda o loop com um evento vazio:
    // o `user_event` marca as janelas e o `about_to_wait` que vem depois roda o frame delas.
    // Um evento por vez na fila: uma saída pesada chama o `wake` a cada pedaço lido, e o
    // winit esvazia a fila inteira antes de responder ao sistema e de rodar o frame.
    let proxy = event_loop.create_proxy();
    let queued = Arc::new(AtomicBool::new(false));
    let waker_queued = Arc::clone(&queued);
    tomni_ui_core::set_waker(move || {
        if !waker_queued.swap(true, Ordering::AcqRel) {
            let _ = proxy.send_event(());
        }
    });
    if let Some(log) = options.stall_log.clone() {
        crate::stall::start(log);
    }
    let mut runner = Runner::new(options, handler, factory, queued);
    let result = event_loop.run_app(&mut runner).map_err(Error::EventLoop);
    // Um pânico não passa daqui: a marca da abertura fica e a próxima sobe o degrau.
    if let Some(guard) = &mut runner.guard {
        guard.finish(result.is_err() || runner.error.is_some());
    }
    tomni_ui_core::clear_waker();
    tomni_ui_core::native::clear_host();
    tomni_ui_core::dialogs::clear_service();
    tomni_ui_core::tray::clear_backend();
    let error = runner.error.take();
    drop(runner);
    windows::reset();
    result?;
    error.map_or(Ok(()), Err)
}

/// O que as janelas dividem: o dispositivo da GPU e os renderizadores.
struct Gpu {
    context: RenderContext,
    /// Um `Renderer` por dispositivo do `context`, criado quando a primeira superfície o usa.
    renderers: Vec<Option<Renderer>>,
    /// O quadro sai do `vello_cpu` em todo adaptador ([`Mode::cpu_raster`]).
    cpu_raster: bool,
}

/// Um arrasto que saiu da janela de origem e está sobre outra janela do processo.
#[derive(Clone, Copy)]
struct Foreign {
    source: WindowHandle,
    target: WindowHandle,
}

struct Runner<H, F> {
    /// O `Handler` da janela principal, até `resumed` criar a janela.
    first: Option<H>,
    factory: F,
    /// As opções da janela principal, base das que abrem depois.
    options: WindowOptions,
    gpu: Gpu,
    /// A guarda da abertura ([`WindowOptions::render_guard`]).
    guard: Option<RenderGuard>,
    /// As janelas abertas, na ordem em que abriram.
    windows: Vec<Janela<H>>,
    foreign: Option<Foreign>,
    error: Option<Error>,
    /// Há um evento do [`tomni_ui_core::wake`] na fila do loop: os outros esperam ele sair.
    wake_queued: Arc<AtomicBool>,
}

/// Uma janela do sistema com o seu `Handler`.
struct Janela<H> {
    handle: WindowHandle,
    handler: H,
    window: Arc<Window>,
    /// Some entre `resumed` e `suspended`: no Android a superfície morre com a atividade em
    /// segundo plano.
    surface: Option<RenderSurface<'static>>,
    canvas: GpuCanvas,
    /// Some quando o adaptador da superfície é um driver em software: ver [`CpuRaster`].
    cpu: Option<CpuRaster>,
    scale: f64,
    /// Tamanho passado ao `handler`, em pixels lógicos.
    logical: (u32, u32),
    pointer: Point,
    /// Arquivos soltos sobre a janela, a entregar ao `handler` de uma vez (o winit manda um
    /// evento por arquivo).
    dropped: Vec<std::path::PathBuf>,
    mouse_pressed: bool,
    /// Dedo que faz as vezes do ponteiro: só o primeiro toque conta.
    touch: Option<u64>,
    modifiers: Modifiers,
    clicks: ClickCounter,
    blink: Blink,
    /// Uma thread chamou [`tomni_ui_core::wake`]: o próximo `about_to_wait` roda o frame, que
    /// aplica o que ela mudou (signals, efeitos) mesmo sem nada animando.
    woken: bool,
    /// Falhas seguidas ao pegar a textura da superfície.
    failures: SurfaceFailures,
    /// A superfície se perdeu e não deu para recriar: tenta de novo no próximo redesenho.
    lost: bool,
    /// O rastro dos quadros (`TOMNI_UI_TRACE_FRAMES`).
    trace: FrameTrace,
    /// A rede de segurança contra a janela que pinta sem parar.
    pacer: Pacer,
    /// Última vez que o tempo foi passado ao `handler` ([`Handler::advance`]).
    clock: Instant,
    title: Option<String>,
    text_input: Option<TextInput>,
    dark: Option<bool>,
    /// Forma do ponteiro do sistema agora: as setas sobre as bordas que redimensionam e a
    /// que o widget embaixo pede.
    cursor: CursorIcon,
    /// Janela transparente, com o Mica atrás.
    transparent: bool,
    #[cfg(target_os = "android")]
    safe_area: Insets,
}

impl<H: Handler, F: FnMut(OpenWindow) -> Option<H>> Runner<H, F> {
    fn new(options: WindowOptions, handler: H, factory: F, wake_queued: Arc<AtomicBool>) -> Self {
        let guard = options.render_guard.clone().map(RenderGuard::open);
        let mode = guard.as_ref().map_or(Mode::default(), RenderGuard::mode);
        Self {
            first: Some(handler),
            factory,
            options,
            gpu: Gpu {
                context: mode.context(),
                renderers: Vec::new(),
                cpu_raster: mode.cpu_raster(),
            },
            guard,
            windows: Vec::new(),
            foreign: None,
            error: None,
            wake_queued,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: Error) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn index(&self, id: WindowId) -> Option<usize> {
        self.windows.iter().position(|j| j.window.id() == id)
    }

    fn index_of(&self, handle: WindowHandle) -> Option<usize> {
        self.windows.iter().position(|j| j.handle == handle)
    }

    /// Cria a janela `handle` com `handler` e põe na lista, pronta para desenhar.
    fn create(
        &mut self,
        event_loop: &ActiveEventLoop,
        handle: WindowHandle,
        handler: H,
        options: WindowOptions,
        position: Option<(i32, i32)>,
    ) -> Result<(), Error> {
        let mut janela = Janela::open(event_loop, handle, handler, options, position)?;
        if handle == WindowHandle::main() {
            tomni_ui_core::native::set_host(Rc::new(janela.window.clone()));
            janela.share_dialogs();
            #[cfg(not(target_os = "android"))]
            crate::tray::install();
        }
        janela.make_surface(&mut self.gpu)?;
        let dark = janela.system_dark();
        janela.set_dark(dark);
        janela.sync_size(&mut self.gpu);
        janela.update(false);
        self.windows.push(janela);
        Ok(())
    }

    /// Atende o que o app pediu às janelas: abrir, fechar e focar.
    fn process_requests(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(dark) = windows::take_theme_request() {
            for janela in &mut self.windows {
                windows::set_current(janela.handle);
                janela.handler.set_theme_override(dark);
                if dark.is_none() {
                    // Volta ao do sistema, mesmo que ele seja o que a janela já mostrava.
                    janela.dark = None;
                    let system = janela.system_dark();
                    janela.set_dark(system);
                }
                janela.update(false);
            }
        }
        loop {
            let opens = windows::take_open_requests();
            let closes = windows::take_close_requests();
            let visibility = windows::take_visibility_requests();
            let focuses = windows::take_focus_requests();
            let attentions = windows::take_attention_requests();
            if opens.is_empty()
                && closes.is_empty()
                && visibility.is_empty()
                && focuses.is_empty()
                && attentions.is_empty()
            {
                return;
            }
            for request in opens {
                let handle = request.handle;
                let mut options = self.options.clone();
                if !request.title.is_empty() {
                    options.title.clone_from(&request.title);
                }
                if let Some((width, height)) = request.size {
                    (options.width, options.height) = (width, height);
                }
                let position = request.position;
                let Some(handler) = windows::enter(handle, || (self.factory)(request)) else {
                    windows::finish_close(handle);
                    continue;
                };
                options.title_bar = handler.has_title_bar();
                if let Err(e) = self.create(event_loop, handle, handler, options, position) {
                    self.fail(event_loop, e);
                    return;
                }
            }
            for handle in closes {
                self.request_close(event_loop, handle);
            }
            for (handle, visible) in visibility {
                if let Some(i) = self.index_of(handle) {
                    self.windows[i].set_visible(visible);
                }
            }
            for handle in focuses {
                if let Some(i) = self.index_of(handle) {
                    self.windows[i].window.focus_window();
                }
            }
            for handle in attentions {
                // Na janela que tem o foco o aviso não faz sentido (e alguns sistemas o deixam
                // aceso até a janela perder e ganhar o foco).
                if let Some(i) = self.index_of(handle)
                    && !self.windows[i].window.has_focus()
                {
                    self.windows[i]
                        .window
                        .request_user_attention(Some(UserAttentionType::Informational));
                }
            }
        }
    }

    /// O sistema ou o app pediu para fechar `handle`: o app pode recusar.
    fn request_close(&mut self, event_loop: &ActiveEventLoop, handle: WindowHandle) {
        if windows::confirm_close(handle) {
            self.close(event_loop, handle);
        }
    }

    /// Fecha `handle` (e todas, sendo a principal) e encerra o app quando não sobra janela.
    fn close(&mut self, event_loop: &ActiveEventLoop, handle: WindowHandle) {
        let closed = windows::finish_close(handle);
        if let Some(foreign) = self.foreign
            && (closed.contains(&foreign.source) || closed.contains(&foreign.target))
        {
            self.foreign = None;
        }
        self.windows.retain(|j| !closed.contains(&j.handle));
        if handle == WindowHandle::main() || self.windows.is_empty() {
            event_loop.exit();
        }
    }

    /// Janela da tela (em pixels físicos) que contém `screen`, fora a de índice `except`, e o
    /// ponto local dela, em pixels lógicos. Sem posição das janelas (Wayland), não acha.
    fn window_at(&self, screen: (f64, f64), except: usize) -> Option<(usize, Point)> {
        self.windows
            .iter()
            .enumerate()
            .rev()
            .find_map(|(j, janela)| {
                if j == except {
                    return None;
                }
                let origin = janela.window.inner_position().ok()?;
                let size = janela.window.inner_size();
                let (x, y) = (
                    screen.0 - f64::from(origin.x),
                    screen.1 - f64::from(origin.y),
                );
                let inside =
                    x >= 0.0 && y >= 0.0 && x < f64::from(size.width) && y < f64::from(size.height);
                inside.then(|| (j, Point::new(x / janela.scale, y / janela.scale)))
            })
    }

    /// Leva o arrasto que começou na janela `source` às outras: enquanto o ponteiro está fora
    /// dela, a janela embaixo recebe o `drag_over`; ao soltar (`release`), recebe o `drop`, ou,
    /// se não há janela embaixo, a origem recebe o `drop_outside` (o tear-off).
    fn cross_drag(&mut self, source: usize, release: bool) {
        let Some(data) = self.windows[source].handler.drag_data() else {
            return;
        };
        let origin = self.windows[source].handle;
        let janela = &self.windows[source];
        let (width, height) = (f64::from(janela.logical.0), f64::from(janela.logical.1));
        let pointer = janela.pointer;
        let inside =
            pointer.x >= 0.0 && pointer.y >= 0.0 && pointer.x < width && pointer.y < height;
        if inside {
            self.leave_foreign();
            return;
        }
        let screen = janela.window.inner_position().ok().map(|origin| {
            (
                f64::from(origin.x) + pointer.x * janela.scale,
                f64::from(origin.y) + pointer.y * janela.scale,
            )
        });
        let over = screen.and_then(|screen| self.window_at(screen, source));
        match (over, release) {
            (Some((target, local)), false) => {
                let handle = self.windows[target].handle;
                if self.foreign.is_some_and(|f| f.target != handle) {
                    self.leave_foreign();
                }
                self.foreign = Some(Foreign {
                    source: origin,
                    target: handle,
                });
                windows::enter(handle, || {
                    self.windows[target].handler.foreign_drag_over(&data, local)
                });
                self.windows[target].update(false);
            }
            (None, false) => self.leave_foreign(),
            (Some((target, local)), true) => {
                self.leave_foreign();
                let handle = self.windows[target].handle;
                windows::enter(handle, || {
                    self.windows[target].handler.foreign_drop(data, local)
                });
                windows::enter(origin, || self.windows[source].handler.cancel_drag());
                self.windows[target].update(false);
            }
            (None, true) => {
                self.leave_foreign();
                let janela = &self.windows[source];
                let at = screen.map(|(x, y)| {
                    (
                        (x / janela.scale).round() as i32,
                        (y / janela.scale).round() as i32,
                    )
                });
                windows::enter(origin, || self.windows[source].handler.drop_outside(at));
            }
        }
    }

    /// O arrasto deixou de passar por outra janela: ela apaga a marca.
    fn leave_foreign(&mut self) {
        let Some(foreign) = self.foreign.take() else {
            return;
        };
        if let Some(target) = self.index_of(foreign.target) {
            windows::enter(foreign.target, || {
                self.windows[target].handler.foreign_drag_leave();
            });
            self.windows[target].update(false);
        }
    }

    /// Se o arrasto que passava por outra janela acabou (Esc, toque cancelado), ela apaga a
    /// marca.
    fn sync_foreign(&mut self) {
        let Some(foreign) = self.foreign else {
            return;
        };
        let dragging = self
            .index_of(foreign.source)
            .is_some_and(|i| self.windows[i].handler.drag_data().is_some());
        if !dragging {
            self.leave_foreign();
        }
    }
}

impl<H: Handler> Janela<H> {
    /// Cria a janela do sistema (sem a superfície).
    fn open(
        event_loop: &ActiveEventLoop,
        handle: WindowHandle,
        mut handler: H,
        options: WindowOptions,
        position: Option<(i32, i32)>,
    ) -> Result<Self, Error> {
        handler.set_clipboard(crate::system_clipboard());
        // O tamanho pedido é o desejado: se não cabe no monitor, a janela abre menor e
        // centralizada (M65).
        let (size, position) = fit_size_and_position(event_loop, &options, position);
        let mut attributes = Window::default_attributes()
            .with_title(&options.title)
            .with_inner_size(LogicalSize::new(size.0, size.1))
            .with_transparent(options.backdrop == Backdrop::Mica)
            .with_visible(!handle.is_hidden());
        // O Wayland não deixa o app escolher onde a janela abre, e o winit ignora.
        if let Some((x, y)) = position {
            attributes = attributes.with_position(LogicalPosition::new(x, y));
        }
        let attributes = identity_attributes(event_loop, attributes, &options);
        let (attributes, chrome) = if options.title_bar {
            chrome_attributes(attributes)
        } else {
            (attributes, WindowChrome::System)
        };
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(Error::Window)?,
        );
        if options.title_bar {
            handler.set_chrome(chrome);
        }
        handler.set_maximized(window.is_maximized());
        // O Wayland não tem como criar a janela escondida: ela abre minimizada.
        if handle.is_hidden() && window.is_visible().is_none() {
            window.set_minimized(true);
        }
        let transparent = options.backdrop == Backdrop::Mica && apply_mica(&window);
        if transparent {
            handler.set_transparent(true);
        }
        Ok(Self {
            handle,
            handler,
            scale: window.scale_factor(),
            window,
            surface: None,
            canvas: GpuCanvas::new(1, 1, 1.0),
            cpu: None,
            logical: (0, 0),
            pointer: Point::ZERO,
            dropped: Vec::new(),
            mouse_pressed: false,
            touch: None,
            modifiers: Modifiers::NONE,
            clicks: ClickCounter::new(),
            blink: Blink::new(),
            woken: false,
            failures: SurfaceFailures::default(),
            lost: false,
            trace: FrameTrace::new(Instant::now()),
            pacer: Pacer::new(Instant::now()),
            clock: Instant::now(),
            title: None,
            text_input: None,
            dark: None,
            cursor: CursorIcon::Default,
            transparent,
            #[cfg(target_os = "android")]
            safe_area: Insets::ZERO,
        })
    }

    /// Esconde ou mostra a janela ([`WindowHandle::hide`] e [`WindowHandle::show`]). Mostrar traz à
    /// frente e foca. O Wayland não tem `set_visible` (`is_visible` devolve `None`): lá esconder é
    /// minimizar, e mostrar é desminimizar e focar.
    fn set_visible(&self, visible: bool) {
        if self.window.is_visible().is_none() {
            self.window.set_minimized(!visible);
        } else {
            self.window.set_visible(visible);
            if visible {
                self.window.set_minimized(false);
            }
        }
        if visible {
            self.window.focus_window();
        }
    }

    /// O seletor de arquivo passa a abrir sobre esta janela.
    fn share_dialogs(&self) {
        #[cfg(not(target_os = "android"))]
        tomni_ui_core::dialogs::set_service(Rc::new(crate::file_dialog::SystemFileDialogs::new(
            self.window.clone(),
        )));
    }

    fn logical(&self, position: PhysicalPosition<f64>) -> Point {
        Point::new(position.x / self.scale, position.y / self.scale)
    }

    /// Cria a superfície da janela.
    fn make_surface(&mut self, gpu: &mut Gpu) -> Result<(), Error> {
        self.scale = self.window.scale_factor();
        let size = self.window.inner_size();
        let surface = pollster::block_on(gpu.context.create_surface(
            self.window.clone(),
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .map_err(Error::Render)?;
        let mut surface = surface;
        if self.transparent {
            let adapter = gpu.context.devices[surface.dev_id].adapter();
            let modes = surface.surface.get_capabilities(adapter).alpha_modes;
            if modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
                surface.config.alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;
                gpu.context.configure_surface(&surface);
            }
        }
        let dev = surface.dev_id;
        let software = gpu.cpu_raster
            || gpu.context.devices[dev].adapter().get_info().device_type == wgpu::DeviceType::Cpu;
        self.cpu = software.then(|| self.cpu.take().unwrap_or_else(CpuRaster::new));
        if gpu.renderers.len() <= dev {
            gpu.renderers.resize_with(dev + 1, || None);
        }
        if !software && gpu.renderers[dev].is_none() {
            let device = &gpu.context.devices[dev].device;
            gpu.renderers[dev] = Some(new_renderer(device).map_err(Error::Gpu)?);
        }
        self.surface = Some(surface);
        Ok(())
    }

    /// Acompanha o tamanho e o DPI da janela.
    fn sync_size(&mut self, gpu: &mut Gpu) {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            // Minimizada.
            return;
        }
        if let Some(surface) = &mut self.surface
            && (surface.config.width, surface.config.height) != (size.width, size.height)
        {
            gpu.context.resize_surface(surface, size.width, size.height);
        }
        let logical = (
            (f64::from(size.width) / self.scale).round() as u32,
            (f64::from(size.height) / self.scale).round() as u32,
        );
        if logical != self.logical {
            self.logical = logical;
            self.handler.resize(logical.0, logical.1);
        }
        self.publish_geometry();
        self.sync_safe_area();
        self.request_redraw(Reason::Event);
    }

    /// Pede um quadro por `reason`. Limitada pelo [`Pacer`] e antes do horário, o pedido fica
    /// retido e sai no horário (no `about_to_wait`) ou na próxima entrada.
    fn request_redraw(&mut self, reason: Reason) {
        if self.pacer.allow(Instant::now()) {
            self.trace.request(reason);
            self.window.request_redraw();
        } else {
            self.pacer.hold(reason);
        }
    }

    /// A pessoa mexeu na janela: o [`Pacer`] devolve o ritmo normal e o quadro retido sai já.
    fn input(&mut self, now: Instant) {
        let (held, was_limited) = self.pacer.input(now);
        if let Some(reason) = held {
            self.request_redraw(reason);
        }
        if was_limited && self.trace.on() {
            eprintln!(
                "tomni-ui: {:?} recebeu entrada; ritmo normal de volta",
                self.handle
            );
        }
    }

    /// Um quadro foi pintado (`presented`: chegou à tela). Conta no rastro e no [`Pacer`] e,
    /// se o limite acabou de entrar, escreve o aviso no stderr.
    fn painted(&mut self, now: Instant, presented: bool) {
        let reason = self.trace.frame(presented);
        if let Some(episode) = self.pacer.frame(now, reason) {
            let deadline = if episode.has_deadline() {
                self.handler.deadline_reason()
            } else {
                None
            };
            let line = episode.line(&format!("{:?}", self.handle), deadline.as_deref());
            crate::stall::note(&line);
            eprintln!("{line}");
        }
    }

    /// Conta ao app onde a janela está e o tamanho dela (`WindowHandle::geometry`).
    fn publish_geometry(&self) {
        if self.logical.0 == 0 || self.logical.1 == 0 {
            return;
        }
        let position = self.window.inner_position().ok().map(|origin| {
            (
                (f64::from(origin.x) / self.scale).round() as i32,
                (f64::from(origin.y) / self.scale).round() as i32,
            )
        });
        windows::set_geometry(
            self.handle,
            WindowGeometry {
                position,
                size: self.logical,
            },
        );
    }

    /// Safe area pelo `content_rect` (só no Android).
    fn sync_safe_area(&mut self) {
        #[cfg(target_os = "android")]
        {
            use winit::platform::android::WindowExtAndroid;
            let window = &self.window;
            let rect = window.content_rect();
            let size = window.inner_size();
            if rect.right <= rect.left || rect.bottom <= rect.top {
                return;
            }
            let (width, height) = (size.width as i32, size.height as i32);
            let px = |v: i32| f64::from(v.max(0)) / self.scale;
            let insets = Insets::new(
                px(rect.top),
                px(width - rect.right),
                px(height - rect.bottom),
                px(rect.left),
            );
            if insets != self.safe_area {
                self.safe_area = insets;
                self.handler.set_safe_area(insets);
            }
        }
    }

    /// Tema do sistema.
    fn system_dark(&self) -> bool {
        #[cfg(target_os = "android")]
        {
            use winit::platform::android::WindowExtAndroid;
            use winit::platform::android::activity::ndk::configuration::UiModeNight;
            return self.window.config().ui_mode_night() == UiModeNight::Yes;
        }
        #[cfg_attr(target_os = "android", allow(unreachable_code))]
        {
            self.window.theme() == Some(Theme::Dark)
        }
    }

    fn set_dark(&mut self, dark: bool) {
        if self.dark != Some(dark) {
            self.dark = Some(dark);
            self.handler.set_dark(dark);
        }
    }

    /// Depois de cada evento: roda o frame, pede o redesenho se preciso e acompanha o título,
    /// o IME e o pisca do cursor. `input` é uma tecla, clique ou toque, que acende o cursor.
    fn update(&mut self, input: bool) {
        windows::set_current(self.handle);
        let now = Instant::now();
        if input && self.blink.running() {
            self.blink.restart(now);
            self.handler.set_caret_visible(self.blink.visible());
        }
        let mut dirty = self.handler.frame();
        let text_input = self.handler.text_input();
        match text_input {
            Some(_) if !self.blink.running() => {
                self.blink.restart(now);
                // Na janela inativa, o campo que acabou de ganhar o foco já começa apagado.
                if !self.blink.visible() {
                    self.handler.set_caret_visible(false);
                    dirty |= self.handler.frame();
                }
            }
            None if self.blink.running() => {
                self.blink.stop();
                self.handler.set_caret_visible(true);
                dirty |= self.handler.frame();
            }
            _ => {}
        }
        // Um quadro que não chegou à tela (a superfície falhou) sai no próximo evento.
        if dirty || self.failures.stale() {
            self.request_redraw(Reason::Event);
        }
        self.sync_ime(text_input);
        let title = self.handler.title();
        if title != self.title {
            if let Some(title) = &title {
                self.window.set_title(title);
                windows::set_title(self.handle, title);
            }
            self.title = title;
        }
    }

    /// Atende os pedidos da `TitleBar`. Arrastar e redimensionar passam o botão ao sistema, que
    /// não devolve o soltar: o gesto termina aqui. Devolve se a janela pediu para fechar.
    fn apply_requests(&mut self) -> bool {
        let requests = self.handler.window_requests();
        let window = self.window.clone();
        let mut close = false;
        for request in requests {
            match request {
                WindowRequest::Minimize => window.set_minimized(true),
                WindowRequest::ToggleMaximize => window.set_maximized(!window.is_maximized()),
                WindowRequest::Close => close = true,
                WindowRequest::Drag => {
                    self.release_pointer();
                    if window.drag_window().is_err() {
                        // O sistema não deixa a janela ser arrastada pelo app: a moldura volta.
                        window.set_decorations(true);
                        self.handler.set_chrome(WindowChrome::System);
                    }
                }
                WindowRequest::Resize(edge) => {
                    self.release_pointer();
                    let _ = window.drag_resize_window(direction(edge));
                }
            }
        }
        close
    }

    /// O sistema ficou com o botão pressionado (arrastar ou redimensionar a janela).
    fn release_pointer(&mut self) {
        self.mouse_pressed = false;
        self.handler.pointer_cancel();
    }

    /// Mostra as setas de redimensionar sobre as bordas da janela sem moldura e, fora delas,
    /// a forma que o widget sob o ponteiro pede (o divisor do `Split`).
    fn sync_cursor(&mut self) {
        let icon = match self.handler.resize_edge(self.pointer) {
            Some(edge) => cursor(edge),
            None => match self.handler.cursor(self.pointer) {
                Cursor::Default => CursorIcon::Default,
                Cursor::EwResize => CursorIcon::EwResize,
                Cursor::NsResize => CursorIcon::NsResize,
                Cursor::Hand => CursorIcon::Pointer,
            },
        };
        if icon == self.cursor {
            return;
        }
        self.cursor = icon;
        self.window.set_cursor(icon);
    }

    /// Passa ao `handler` o tempo desde a última vez, se ele estiver contando algo. Retorna se
    /// passou.
    fn tick(&mut self, now: Instant) -> bool {
        let dt = now.saturating_duration_since(self.clock);
        self.clock = now;
        if self.handler.deadline().is_some() {
            self.handler.advance(dt);
            true
        } else {
            false
        }
    }

    fn sync_ime(&mut self, text_input: Option<TextInput>) {
        if text_input == self.text_input {
            return;
        }
        let window = &self.window;
        // Nas senhas o IME fica desligado e o texto chega como teclas. No Android ligar o IME
        // é o que abre o teclado virtual, então ele fica ligado e só o tipo muda.
        let allowed = |input: Option<TextInput>| {
            input
                .is_some_and(|i| i.purpose != InputPurpose::Password || cfg!(target_os = "android"))
        };
        if allowed(text_input) != allowed(self.text_input) {
            window.set_ime_allowed(allowed(text_input));
        }
        if let Some(input) = text_input {
            if self.text_input.map(|i| i.purpose) != Some(input.purpose) {
                window.set_ime_purpose(match input.purpose {
                    InputPurpose::Normal => ImePurpose::Normal,
                    InputPurpose::Password => ImePurpose::Password,
                });
            }
            let area = input.area;
            window.set_ime_cursor_area(
                LogicalPosition::new(area.x0, area.y0),
                LogicalSize::new(area.width(), area.height()),
            );
        }
        self.text_input = text_input;
    }

    /// Pinta o quadro e apresenta; devolve se chegou à tela. A textura da superfície vem antes
    /// da pintura: sem ela (janela coberta, superfície que não volta), o quadro não custa nada.
    fn redraw(&mut self, gpu: &mut Gpu) -> Result<bool, Error> {
        windows::set_current(self.handle);
        self.sync_safe_area();
        {
            let _quadro = stages::stage("quadro");
            self.handler.frame();
        }
        let texture = {
            let _textura = stages::stage("textura");
            self.acquire(gpu)
        };
        let Some(texture) = texture else {
            return Ok(false);
        };
        let Some(surface) = &mut self.surface else {
            return Ok(false);
        };
        let (width, height) = (surface.config.width, surface.config.height);
        let device = &gpu.context.devices[surface.dev_id];
        let pintura = stages::stage("pintura");
        let source = if let Some(cpu) = &mut self.cpu {
            cpu.render(&self.handler, device, (width, height), self.scale)
        } else {
            self.canvas
                .reset(self.logical.0.max(1), self.logical.1.max(1), self.scale);
            self.handler.render(&mut self.canvas);
            let renderer = gpu.renderers[surface.dev_id]
                .as_mut()
                .expect("o renderer é criado junto com a superfície");
            renderer
                .render_to_texture(
                    &device.device,
                    &device.queue,
                    self.canvas.scene(),
                    &surface.target_view,
                    &render_params(width, height, self.handler.background()),
                )
                .map_err(Error::Render)?;
            &surface.target_view
        };
        drop(pintura);
        let _apresentacao = stages::stage("apresentação");
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tomni-ui janela"),
            });
        surface
            .blitter
            .copy(&device.device, &mut encoder, source, &view);
        device.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        device.queue.present(texture);
        Ok(true)
    }

    /// Pega a textura da superfície para o próximo quadro. A desatualizada é reconfigurada e a
    /// perdida, recriada, com uma nova tentativa na hora. Se falhar de novo, só a primeira falha
    /// seguida pede outro redesenho ([`SurfaceFailures`]); o quadro fica pendente para o próximo
    /// evento.
    fn acquire(&mut self, gpu: &mut Gpu) -> Option<wgpu::SurfaceTexture> {
        if self.surface.is_none() && self.lost {
            // A superfície se perdeu e a recriação falhou da outra vez.
            match self.make_surface(gpu) {
                Ok(()) => self.lost = false,
                Err(_) => return None,
            }
        }
        for attempt in 0..2 {
            let surface = self.surface.as_mut()?;
            match surface.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(texture)
                | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                    self.failures.succeeded();
                    return Some(texture);
                }
                // Janela coberta ou GPU ocupada: fica para o próximo evento ou exposição.
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    self.failures.skipped();
                    return None;
                }
                wgpu::CurrentSurfaceTexture::Lost if attempt == 0 => {
                    self.surface = None;
                    if let Err(e) = self.make_surface(gpu) {
                        eprintln!("tomni-ui: a superfície da janela se perdeu e não voltou: {e}");
                        self.lost = true;
                        break;
                    }
                }
                _ if attempt == 0 => gpu.context.configure_surface(surface),
                _ => {}
            }
        }
        if self.failures.failed() {
            self.request_redraw(Reason::Surface);
        }
        None
    }
}

/// Rasterização em CPU para quando o adaptador é um driver em software (o WARP do Windows sem
/// driver de vídeo, numa VM ou numa sessão remota; o llvmpipe do Linux).
///
/// O `vello` cria os shaders sem as verificações de limite do `naga`, e o WARP derruba o
/// processo com uma escrita fora do buffer que uma GPU de verdade ignora. Aqui o quadro sai
/// do `vello_cpu`, o mesmo dos snapshots, e o `wgpu` só copia os pixels para a janela, sem
/// compute shader. Numa CPU, o `vello_cpu` também é mais rápido que os shaders emulados.
struct CpuRaster {
    canvas: CpuCanvas,
    /// Textura que recebe os pixels, recriada quando o tamanho muda.
    texture: Option<(wgpu::Texture, wgpu::TextureView)>,
}

impl CpuRaster {
    fn new() -> Self {
        Self {
            canvas: CpuCanvas::scaled(1, 1, 1.0),
            texture: None,
        }
    }

    /// Desenha o quadro e devolve a textura com os pixels, de `width` × `height` pixels físicos.
    fn render(
        &mut self,
        handler: &impl Handler,
        device: &tomni_ui_render::vello::util::DeviceHandle,
        (width, height): (u32, u32),
        scale: f64,
    ) -> &wgpu::TextureView {
        // O `vello_cpu` mede em u16; a janela não passa disso.
        let pixels = |n: u32| u16::try_from(n).unwrap_or(u16::MAX).max(1);
        let (w, h) = (pixels(width), pixels(height));
        self.canvas.reset(w, h, scale);
        handler.render(&mut self.canvas);
        let pixmap = self.canvas.render(handler.background());

        let size = wgpu::Extent3d {
            width: w.into(),
            height: h.into(),
            depth_or_array_layers: 1,
        };
        if self
            .texture
            .as_ref()
            .is_none_or(|(texture, _)| texture.size() != size)
        {
            let texture = device.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("tomni-ui quadro da CPU"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.texture = Some((texture, view));
        }
        let (texture, view) = self.texture.as_ref().expect("criada acima");
        device.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixmap.data_as_u8_slice(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * u32::from(w)),
                rows_per_image: Some(h.into()),
            },
            size,
        );
        view
    }
}

impl<H: Handler, F: FnMut(OpenWindow) -> Option<H>> ApplicationHandler for Runner<H, F> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.windows.is_empty() {
            let Some(handler) = self.first.take() else {
                return;
            };
            let options = self.options.clone();
            if let Err(e) = self.create(event_loop, WindowHandle::main(), handler, options, None) {
                self.fail(event_loop, e);
            }
            return;
        }
        // Voltou do segundo plano (Android): as superfícies foram descartadas.
        for i in 0..self.windows.len() {
            let janela = &mut self.windows[i];
            if let Err(e) = janela.make_surface(&mut self.gpu) {
                self.fail(event_loop, e);
                return;
            }
            let dark = janela.system_dark();
            janela.set_dark(dark);
            janela.sync_size(&mut self.gpu);
            janela.update(false);
        }
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        for janela in &mut self.windows {
            janela.surface = None;
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(i) = self.index(id) else {
            return;
        };
        let _etapa = stages::stage(if matches!(event, WindowEvent::RedrawRequested) {
            "desenho"
        } else {
            "evento"
        });
        let now = Instant::now();
        let handle = self.windows[i].handle;
        windows::set_current(handle);
        // O evento pode começar uma contagem: ela vale a partir de agora.
        self.windows[i].tick(now);
        let janela = &mut self.windows[i];
        if is_input(&event) {
            janela.input(now);
        }
        let mut released = false;
        let mut moved = false;
        let input = match event {
            WindowEvent::CloseRequested => {
                self.request_close(event_loop, handle);
                return;
            }
            WindowEvent::RedrawRequested => {
                // Limitada e antes do horário (um pedido do sistema): o quadro fica retido.
                if !janela.pacer.allow(now) {
                    let reason = janela.trace.take().unwrap_or(Reason::System);
                    janela.pacer.hold(reason);
                    return;
                }
                match janela.redraw(&mut self.gpu) {
                    Ok(presented) => {
                        if presented && let Some(guard) = &mut self.guard {
                            guard.shown();
                        }
                        janela.painted(now, presented)
                    }
                    Err(e) => self.fail(event_loop, e),
                }
                return;
            }
            WindowEvent::Resized(_) => {
                janela.sync_size(&mut self.gpu);
                janela.handler.set_maximized(janela.window.is_maximized());
                false
            }
            WindowEvent::Moved(_) => {
                janela.publish_geometry();
                return;
            }
            WindowEvent::DroppedFile(path) => {
                janela.dropped.push(path);
                false
            }
            WindowEvent::Focused(focused) => {
                janela.handler.set_window_active(focused);
                // Na janela inativa o cursor fica apagado e parado; o `update` pinta a troca.
                if janela.blink.set_active(focused, now) {
                    janela.handler.set_caret_visible(janela.blink.visible());
                }
                if focused {
                    janela.share_dialogs();
                    windows::set_active(handle);
                }
                false
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                janela.scale = scale_factor;
                janela.sync_size(&mut self.gpu);
                // No Android, a mudança de configuração (tema noturno inclusive) chega aqui.
                let dark = janela.system_dark();
                janela.set_dark(dark);
                false
            }
            WindowEvent::ThemeChanged(theme) => {
                janela.set_dark(theme == Theme::Dark);
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                janela.pointer = janela.logical(position);
                janela
                    .handler
                    .pointer_move(janela.pointer, PointerKind::Mouse);
                janela.sync_cursor();
                moved = janela.mouse_pressed;
                false
            }
            WindowEvent::CursorLeft { .. } => {
                // Com o botão pressionado o sistema continua mandando o ponteiro (captura), e o
                // arraste segue fora da janela.
                if !janela.mouse_pressed {
                    janela.handler.pointer_cancel();
                }
                false
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // O `winit` dá positivo para rolar para cima; o core, para mostrar o que está
                // mais abaixo.
                let delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        Vec2::new(f64::from(-x), f64::from(-y)) * WHEEL_LINE
                    }
                    MouseScrollDelta::PixelDelta(p) => -Vec2::new(p.x, p.y) / janela.scale,
                };
                janela.handler.wheel(janela.pointer, delta);
                false
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                match state {
                    ElementState::Pressed => {
                        janela.mouse_pressed = true;
                        let clicks = janela.clicks.press(now, janela.pointer);
                        janela
                            .handler
                            .pointer_down(janela.pointer, PointerKind::Mouse, clicks);
                    }
                    ElementState::Released => {
                        janela.mouse_pressed = false;
                        released = true;
                    }
                }
                true
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                match state {
                    ElementState::Pressed => janela.handler.secondary_down(janela.pointer),
                    ElementState::Released => janela.handler.secondary_up(janela.pointer),
                }
                true
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Middle,
                ..
            } => {
                match state {
                    ElementState::Pressed => janela.handler.middle_down(janela.pointer),
                    ElementState::Released => janela.handler.middle_up(janela.pointer),
                }
                true
            }
            WindowEvent::Touch(touch) => {
                let point = janela.logical(touch.location);
                let ours = janela.touch == Some(touch.id);
                match touch.phase {
                    TouchPhase::Started if janela.touch.is_none() => {
                        janela.touch = Some(touch.id);
                        let clicks = janela.clicks.press(now, point);
                        janela.handler.pointer_move(point, PointerKind::Touch);
                        janela
                            .handler
                            .pointer_down(point, PointerKind::Touch, clicks);
                    }
                    TouchPhase::Moved if ours => {
                        janela.handler.pointer_move(point, PointerKind::Touch);
                    }
                    TouchPhase::Ended if ours => {
                        janela.touch = None;
                        janela.handler.pointer_up(point, PointerKind::Touch);
                    }
                    TouchPhase::Cancelled if ours => {
                        janela.touch = None;
                        janela.handler.pointer_cancel();
                    }
                    _ => return,
                }
                true
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                janela.modifiers = keys::modifiers(modifiers.state());
                janela.handler.set_modifiers(janela.modifiers);
                false
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                let key = keys::key(&event.logical_key);
                match event.state {
                    ElementState::Pressed => {
                        janela.handler.key_down(key);
                        if let Some(text) = keys::text(event.text.as_deref(), janela.modifiers) {
                            janela.handler.type_text(text);
                        }
                    }
                    ElementState::Released => janela.handler.key_up(key),
                }
                true
            }
            WindowEvent::Ime(ime) => {
                match ime {
                    winit::event::Ime::Preedit(text, cursor) => {
                        janela.handler.ime(Ime::Preedit { text, cursor });
                    }
                    winit::event::Ime::Commit(text) => janela.handler.ime(Ime::Commit(text)),
                    // O IME foi desligado no meio de uma composição: ela some.
                    winit::event::Ime::Disabled => janela.handler.ime(Ime::Preedit {
                        text: String::new(),
                        cursor: None,
                    }),
                    winit::event::Ime::Enabled => return,
                }
                true
            }
            _ => return,
        };
        // Um item arrastado que saiu da janela vai para a de baixo, ou vira janela nova; o
        // soltar acontece antes de a janela de origem tratar o botão solto.
        if moved || released {
            self.cross_drag(i, released);
        }
        let janela = &mut self.windows[i];
        if released {
            janela
                .handler
                .pointer_up(janela.pointer, PointerKind::Mouse);
            janela.sync_cursor();
        }
        let close = janela.apply_requests();
        janela.update(input);
        self.sync_foreign();
        if close {
            self.request_close(event_loop, handle);
        }
        self.process_requests(event_loop);
    }

    /// O evento vazio de [`tomni_ui_core::wake`]: marca as janelas, e o `about_to_wait` que vem
    /// depois roda o frame delas. Sem isso, o que uma thread mudou só aparecia no próximo evento
    /// de entrada, quando nada animava.
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, (): ()) {
        let _acordar = stages::stage("acordar");
        // Antes de marcar: o `wake` que vier daqui em diante põe outro evento na fila.
        self.wake_queued.store(false, Ordering::Release);
        for janela in &mut self.windows {
            janela.woken = true;
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let _ciclo = stages::stage("ciclo");
        // O motor dos controles nativos (o GTK do `WebView`) gira no mesmo loop, antes do
        // frame: o que ele avisou entra neste.
        let pump = tomni_ui_core::native::pump();
        let now = Instant::now();
        let pump = pump.map(|left| now + left);
        // Os pedidos feitos fora de um evento (uma thread, um efeito): abrir, fechar, focar.
        self.process_requests(event_loop);
        let mut deadlines: Vec<Instant> = pump.into_iter().collect();
        for janela in &mut self.windows {
            windows::set_current(janela.handle);
            // Limitada pelo `Pacer` e antes do horário: nada de frame; o loop volta no horário,
            // e o tempo que passou chega inteiro ao `Handler`.
            if let Some(slot) = janela.pacer.wait(now) {
                deadlines.push(slot);
                continue;
            }
            if let Some(held) = janela.pacer.release(now) {
                janela.request_redraw(held);
            }
            let mut reason = std::mem::take(&mut janela.woken).then_some(Reason::Event);
            let due = {
                let _prazos = stages::stage("prazos");
                janela.tick(now)
            };
            if due {
                reason = Some(Reason::Deadline);
                janela
                    .trace
                    .note_deadline(|| janela.handler.deadline_reason());
            }
            if !janela.dropped.is_empty() {
                let paths = std::mem::take(&mut janela.dropped);
                let point = janela.pointer;
                if janela.handler.files_dropped(paths, point) {
                    reason = Some(Reason::Event);
                }
            }
            if janela.blink.tick(now) {
                janela.handler.set_caret_visible(janela.blink.visible());
                reason = Some(Reason::Blink);
            }
            if let Some(reason) = reason
                && {
                    let _quadro = stages::stage("quadro");
                    janela.handler.frame()
                }
            {
                janela.request_redraw(reason);
            }
            if janela.trace.on()
                && let Some(line) = janela.trace.flush(now, &format!("{:?}", janela.handle))
            {
                eprintln!("{line}");
            }
            let pacer = &janela.pacer;
            deadlines.extend(janela.blink.deadline().map(|at| pacer.clamp(at)));
            deadlines.extend(
                janela
                    .handler
                    .deadline()
                    .map(|left| pacer.clamp(now + left.max(Duration::from_millis(1)))),
            );
        }
        event_loop.set_control_flow(match deadlines.into_iter().min() {
            Some(deadline) => ControlFlow::WaitUntil(deadline),
            None => ControlFlow::Wait,
        });
    }
}

/// Se o evento é uma entrada da pessoa, que devolve o ritmo normal à janela limitada pelo
/// [`Pacer`]: teclado, mouse (mover, entrar, sair, botão, roda), toque e gestos, a composição
/// do IME, arquivos arrastados, tamanho e foco. O `Ime::Enabled`/`Disabled` não conta: vem do
/// próprio app ligando o IME.
fn is_input(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::DoubleTapGesture { .. }
            | WindowEvent::RotationGesture { .. }
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::Ime(winit::event::Ime::Preedit(..) | winit::event::Ime::Commit(_))
            | WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFile(_)
            | WindowEvent::HoveredFileCancelled
            | WindowEvent::Resized(_)
            | WindowEvent::Focused(_)
    )
}

/// Janela sem a moldura do sistema para a `TitleBar`, e quem desenha os botões da janela.
fn chrome_attributes(attributes: WindowAttributes) -> (WindowAttributes, WindowChrome) {
    #[cfg(target_os = "android")]
    {
        (attributes, WindowChrome::System)
    }
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS;
        // A barra do sistema fica transparente, com os três botões por cima da do app.
        let attributes = attributes
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true);
        (attributes, WindowChrome::MacOs)
    }
    #[cfg(not(any(target_os = "android", target_os = "macos")))]
    {
        (attributes.with_decorations(false), WindowChrome::Custom)
    }
}

/// Põe nos atributos da janela o ícone e o nome (`app_id`) do app (M49).
///
/// O ícone que o `winit` aceita por janela é um só no X11 (`_NET_WM_ICON`, de onde o dock e o
/// alternador de janelas tiram o deles): vai o grande. No Windows, o pequeno fica na moldura e o
/// grande na barra de tarefas. O Wayland não recebe ícone pela janela: ele o acha pelo
/// `app_id`, no atalho `.desktop` do app. Um desenho que não decodifica fica de fora, e a janela
/// abre com o ícone do sistema.
/// O tamanho e a posição lógicos em que a janela abre (M65): o pedido, reduzido ao monitor onde
/// ela vai abrir se não couber. O monitor é o que contém a `position`, quando ela veio; senão, o
/// primário; se o sistema não informa (o Wayland às vezes devolve `None`), o primeiro da lista.
#[cfg(not(target_os = "android"))]
fn fit_size_and_position(
    event_loop: &ActiveEventLoop,
    options: &WindowOptions,
    position: Option<(i32, i32)>,
) -> ((f64, f64), Option<(f64, f64)>) {
    use crate::placement::{MonitorArea, fit_to_monitor};
    let area = |monitor: &winit::monitor::MonitorHandle| {
        let scale = monitor.scale_factor();
        let origin: LogicalPosition<f64> = monitor.position().to_logical(scale);
        let size: LogicalSize<f64> = monitor.size().to_logical(scale);
        MonitorArea {
            x: origin.x,
            y: origin.y,
            width: size.width,
            height: size.height,
        }
    };
    let requested = (f64::from(options.width), f64::from(options.height));
    let position = position.map(|(x, y)| (f64::from(x), f64::from(y)));
    let monitor = position
        .and_then(|point| {
            event_loop
                .available_monitors()
                .map(|m| area(&m))
                .find(|m| m.contains(point))
        })
        .or_else(|| event_loop.primary_monitor().map(|m| area(&m)))
        .or_else(|| event_loop.available_monitors().next().map(|m| area(&m)))
        // Um monitor sem tamanho (alguns Wayland) não serve para a conta.
        .filter(|m| m.width > 0.0 && m.height > 0.0);
    fit_to_monitor(requested, position, monitor)
}

/// No Android a janela é a tela toda: o tamanho pedido é ignorado.
#[cfg(target_os = "android")]
fn fit_size_and_position(
    _event_loop: &ActiveEventLoop,
    options: &WindowOptions,
    position: Option<(i32, i32)>,
) -> ((f64, f64), Option<(f64, f64)>) {
    (
        (f64::from(options.width), f64::from(options.height)),
        position.map(|(x, y)| (f64::from(x), f64::from(y))),
    )
}

fn identity_attributes(
    event_loop: &ActiveEventLoop,
    attributes: WindowAttributes,
    options: &WindowOptions,
) -> WindowAttributes {
    let _ = event_loop;
    let mut attributes = attributes;
    if let Some(icon) = &options.icon {
        let make = |side: u32| {
            let (data, width, height) = crate::icon::rgba(icon.for_side(side), side)?;
            winit::window::Icon::from_rgba(data, width, height).ok()
        };
        #[cfg(windows)]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes = attributes
                .with_window_icon(make(crate::icon::SMALL_SIDE))
                .with_taskbar_icon(make(crate::icon::TASKBAR_SIDE));
        }
        #[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
        {
            use winit::platform::x11::ActiveEventLoopExtX11;
            let side = if event_loop.is_x11() {
                crate::icon::LARGE_SIDE
            } else {
                crate::icon::SMALL_SIDE
            };
            attributes = attributes.with_window_icon(make(side));
        }
        #[cfg(not(any(
            windows,
            all(unix, not(any(target_os = "macos", target_os = "android")))
        )))]
        {
            let _ = make;
        }
    }
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
    if let Some(app_id) = &options.app_id {
        use winit::platform::wayland::WindowAttributesExtWayland;
        use winit::platform::x11::WindowAttributesExtX11;
        attributes = WindowAttributesExtX11::with_name(attributes, app_id, app_id);
        attributes = WindowAttributesExtWayland::with_name(attributes, app_id, app_id);
    }
    attributes
}

/// Liga o Mica do Windows 11. `false` fora do Windows ou se o sistema recusar.
fn apply_mica(window: &Window) -> bool {
    #[cfg(windows)]
    {
        window_vibrancy::apply_mica(window, None).is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        false
    }
}

fn direction(edge: ResizeEdge) -> ResizeDirection {
    match edge {
        ResizeEdge::North => ResizeDirection::North,
        ResizeEdge::South => ResizeDirection::South,
        ResizeEdge::East => ResizeDirection::East,
        ResizeEdge::West => ResizeDirection::West,
        ResizeEdge::NorthEast => ResizeDirection::NorthEast,
        ResizeEdge::NorthWest => ResizeDirection::NorthWest,
        ResizeEdge::SouthEast => ResizeDirection::SouthEast,
        ResizeEdge::SouthWest => ResizeDirection::SouthWest,
    }
}

fn cursor(edge: ResizeEdge) -> CursorIcon {
    match edge {
        ResizeEdge::North => CursorIcon::NResize,
        ResizeEdge::South => CursorIcon::SResize,
        ResizeEdge::East => CursorIcon::EResize,
        ResizeEdge::West => CursorIcon::WResize,
        ResizeEdge::NorthEast => CursorIcon::NeResize,
        ResizeEdge::NorthWest => CursorIcon::NwResize,
        ResizeEdge::SouthEast => CursorIcon::SeResize,
        ResizeEdge::SouthWest => CursorIcon::SwResize,
    }
}
