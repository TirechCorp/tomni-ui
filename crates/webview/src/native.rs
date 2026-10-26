//! O motor do sistema, pelo `wry`: o WebView2 no Windows, o WKWebView no macOS e o WebKitGTK
//! no Linux, como janela filha da janela do app.
//!
//! No Linux, o WebKitGTK precisa do GTK girando no mesmo loop da janela: o motor registra um
//! [`add_pump`](tomni_ui_core::native::add_pump) que processa os eventos do GTK a cada
//! [`PUMP_INTERVAL`](tomni_ui_core::native::PUMP_INTERVAL) enquanto houver um WebView, e pede
//! uma janela X11 (o WebKitGTK não põe uma janela filha numa janela Wayland).

use std::any::Any;
use std::borrow::Cow;
use std::cell::Cell;
use std::rc::Rc;

use tomni_ui_core::kurbo::Rect;
use tomni_ui_core::native::Host;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::http::{Request, Response, header::CONTENT_TYPE};
use wry::{NewWindowResponse, PageLoadEvent, WebViewBuilder};

use crate::backend::{
    APP_ORIGIN, Assets, Backend, Decision, INIT_SCRIPT, Notice, Notices, Policy, app_path,
    ipc_notice, notify,
};

thread_local! {
    /// Quantos WebViews do sistema existem nesta thread: o GTK gira enquanto houver algum.
    static LIVE: Cell<usize> = const { Cell::new(0) };
}

/// Chamado ao criar o widget, antes de a janela abrir: no Linux, pede a janela X11.
pub(crate) fn prepare() {
    #[cfg(target_os = "linux")]
    tomni_ui_core::native::require_x11();
}

/// Se o sistema tem o motor. No Android, o `wry` precisa de uma `Activity` própria, ligada pelo
/// `android_binding!` dele, que o `android-activity` do `winit` não tem: o widget mostra o erro.
#[cfg(target_os = "android")]
fn supported() -> Result<(), String> {
    Err(String::from("o Android ainda não tem suporte"))
}

#[cfg(not(target_os = "android"))]
fn supported() -> Result<(), String> {
    Ok(())
}

/// Origem de `fluent://app` no motor: o WebView2 e o WebView do Android servem os protocolos
/// próprios em `http://<protocolo>.<host>`.
#[cfg(any(target_os = "windows", target_os = "android"))]
const ENGINE_ORIGIN: &str = "http://fluent.app";
#[cfg(not(any(target_os = "windows", target_os = "android")))]
const ENGINE_ORIGIN: &str = APP_ORIGIN;

/// `url` como o motor a entende.
fn to_engine(url: &str) -> String {
    match url.strip_prefix(APP_ORIGIN) {
        Some(rest) => format!("{ENGINE_ORIGIN}{rest}"),
        None => url.to_owned(),
    }
}

/// `url` do motor como o app a vê (`fluent://app/...` em todos os sistemas).
fn from_engine(url: &str) -> String {
    match url.strip_prefix(ENGINE_ORIGIN) {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '?', '#']) => {
            format!("{APP_ORIGIN}{rest}")
        }
        _ => url.to_owned(),
    }
}

/// Abre `url` no programa do sistema (o navegador, o cliente de e-mail).
fn open_external(url: &str) {
    if let Err(error) = open::that_detached(url) {
        eprintln!("tomni-ui: não foi possível abrir {url}: {error}");
    }
}

/// Decodifica os `%XX` de um caminho.
fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = path.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Responde a um pedido a `fluent://app/`.
fn serve(assets: &Assets, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let url = from_engine(&request.uri().to_string());
    let path = app_path(&url).map(percent_decode);
    let asset = path.as_deref().and_then(|path| assets.get(path));
    let response = match asset {
        Some(asset) => Response::builder()
            .header(CONTENT_TYPE, asset.mime)
            .body(Cow::Owned(asset.bytes.to_vec())),
        None => Response::builder()
            .status(404)
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Cow::Borrowed(b"arquivo n\xc3\xa3o encontrado".as_slice())),
    };
    response.unwrap_or_else(|_| Response::new(Cow::Borrowed(b"".as_slice())))
}

/// O WebView do sistema.
pub(crate) struct Native {
    webview: wry::WebView,
    notices: Notices,
    visible: bool,
}

impl Native {
    pub(crate) fn new(
        host: &Host,
        notices: Notices,
        policy: Rc<Policy>,
        assets: Assets,
    ) -> Result<Self, String> {
        supported()?;
        #[cfg(target_os = "linux")]
        gtk_init()?;
        let (ipc, load, title, window) = (
            Rc::clone(&notices),
            Rc::clone(&notices),
            Rc::clone(&notices),
            Rc::clone(&notices),
        );
        let navigation = Rc::clone(&policy);
        // Nasce visível, com 1 × 1 px no canto, e o `place` do mesmo frame esconde até a
        // página carregar: no X11, a janela filha que nasce escondida fica preta ao aparecer
        // (o próprio `wry` avisa que começar escondido pede várias trocas de visibilidade).
        let builder = WebViewBuilder::new()
            .with_visible(true)
            .with_bounds(wry::Rect {
                position: LogicalPosition::new(0.0, 0.0).into(),
                size: LogicalSize::new(1.0, 1.0).into(),
            })
            .with_custom_protocol("fluent".into(), move |_, request| serve(&assets, &request))
            .with_initialization_script(INIT_SCRIPT)
            .with_ipc_handler(move |request: Request<String>| {
                if let Some(notice) = ipc_notice(request.body()) {
                    notify(&ipc, notice);
                }
            })
            .with_on_page_load_handler(move |event, url| {
                let url = from_engine(&url);
                notify(
                    &load,
                    match event {
                        PageLoadEvent::Started => Notice::Started(url),
                        PageLoadEvent::Finished => Notice::Finished(url),
                    },
                );
            })
            .with_document_title_changed_handler(move |text| notify(&title, Notice::Title(text)))
            .with_navigation_handler(move |url| {
                let url = from_engine(&url);
                match navigation.decide(&url) {
                    Decision::Allow => true,
                    Decision::Open => {
                        open_external(&url);
                        false
                    }
                    Decision::Block => false,
                }
            })
            .with_new_window_req_handler(move |url, _| {
                let url = from_engine(&url);
                match policy.decide(&url) {
                    Decision::Allow => notify(&window, Notice::Open(url)),
                    Decision::Open => open_external(&url),
                    Decision::Block => {}
                }
                NewWindowResponse::Deny
            });
        // As ferramentas de desenvolvedor do navegador (o "Inspecionar"), só no build de debug.
        #[cfg(debug_assertions)]
        let builder = builder.with_devtools(true);
        let webview = builder
            .build_as_child(host)
            .map_err(|error| error.to_string())?;
        #[cfg(target_os = "linux")]
        connect_load_failed(&webview, Rc::clone(&notices));
        LIVE.set(LIVE.get() + 1);
        #[cfg(target_os = "linux")]
        tomni_ui_core::native::add_pump("gtk", || {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            LIVE.get() > 0
        });
        Ok(Self {
            webview,
            notices,
            visible: true,
        })
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        LIVE.set(LIVE.get().saturating_sub(1));
    }
}

/// Inicia o GTK na primeira vez, só com o X11.
#[cfg(target_os = "linux")]
fn gtk_init() -> Result<(), String> {
    thread_local! {
        static READY: Cell<bool> = const { Cell::new(false) };
    }
    if READY.get() {
        return Ok(());
    }
    gtk::gdk::set_allowed_backends("x11");
    gtk::init().map_err(|error| format!("o GTK não iniciou: {error}"))?;
    READY.set(true);
    Ok(())
}

/// O WebKitGTK avisa as falhas de carga (o `wry` não): sem rede, endereço que não existe.
/// A página de erro dele fica de fora, porque o widget mostra o próprio aviso.
#[cfg(target_os = "linux")]
fn connect_load_failed(webview: &wry::WebView, notices: Notices) {
    use webkit2gtk::{NetworkError, PolicyError, WebViewExt};
    use wry::WebViewExtUnix;
    webview
        .webview()
        .connect_load_failed(move |_, _, url, error| {
            // Uma carga interrompida por outra, ou barrada pela política de navegação, não é
            // falha da página.
            if error.matches(NetworkError::Cancelled)
                || error.matches(PolicyError::FrameLoadInterruptedByPolicyChange)
            {
                return false;
            }
            notify(
                &notices,
                Notice::Failed {
                    url: from_engine(url),
                    error: error.to_string(),
                },
            );
            true
        });
}

impl Backend for Native {
    fn navigate(&mut self, url: &str) {
        let _ = self.webview.load_url(&to_engine(url));
    }

    fn reload(&mut self) {
        let _ = self.webview.reload();
    }

    fn back(&mut self) {
        let _ = self.webview.go_back();
    }

    fn forward(&mut self) {
        let _ = self.webview.go_forward();
    }

    fn eval(&mut self, script: &str) {
        let _ = self.webview.evaluate_script(script);
    }

    fn history(&self) -> (bool, bool) {
        (
            self.webview.can_go_back().unwrap_or(false),
            self.webview.can_go_forward().unwrap_or(false),
        )
    }

    fn place(&mut self, rect: Option<Rect>) {
        match rect {
            Some(rect) => {
                let _ = self.webview.set_bounds(wry::Rect {
                    position: LogicalPosition::new(rect.x0, rect.y0).into(),
                    size: LogicalSize::new(rect.width(), rect.height()).into(),
                });
                if !self.visible {
                    self.visible = true;
                    let _ = self.webview.set_visible(true);
                }
            }
            None => {
                if self.visible {
                    self.visible = false;
                    let _ = self.webview.set_visible(false);
                }
            }
        }
    }

    fn set_dark(&mut self, dark: bool) {
        #[cfg(target_os = "linux")]
        if let Some(settings) = gtk::Settings::default() {
            use gtk::prelude::GtkSettingsExt;
            settings.set_gtk_application_prefer_dark_theme(dark);
        }
        #[cfg(target_os = "windows")]
        {
            use wry::WebViewExtWindows;
            let _ = self.webview.set_theme(if dark {
                wry::Theme::Dark
            } else {
                wry::Theme::Light
            });
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let _ = dark;
    }

    fn focus(&mut self) {
        let _ = self.webview.focus();
    }

    fn unfocus(&mut self) {
        let _ = self.webview.focus_parent();
    }

    fn capture(&mut self) {
        #[cfg(target_os = "linux")]
        capture(&self.webview, Rc::clone(&self.notices));
        #[cfg(not(target_os = "linux"))]
        notify(&self.notices, Notice::Capture(None));
    }

    fn native(&self) -> bool {
        true
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Captura a parte visível da página. O WebKitGTK entrega BGRA pré-multiplicado (o ARGB32 do
/// cairo, em little-endian); o widget quer RGBA.
#[cfg(target_os = "linux")]
fn capture(webview: &wry::WebView, notices: Notices) {
    use std::sync::Arc;

    use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
    use wry::WebViewExtUnix;

    use crate::backend::Capture;

    webview.webview().snapshot(
        SnapshotRegion::Visible,
        SnapshotOptions::NONE,
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let capture = result
                .ok()
                .and_then(|surface| gtk::cairo::ImageSurface::try_from(surface).ok())
                .and_then(|image| {
                    let width = u32::try_from(image.width()).ok()?;
                    let height = u32::try_from(image.height()).ok()?;
                    let stride = usize::try_from(image.stride()).ok()?;
                    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
                    image
                        .with_data(|data| {
                            for row in data.chunks(stride).take(height as usize) {
                                for px in row[..width as usize * 4].chunks_exact(4) {
                                    let px: [u8; 4] = [px[0], px[1], px[2], px[3]];
                                    let [b, g, r, a] = if cfg!(target_endian = "little") {
                                        px
                                    } else {
                                        let [a, r, g, b] = px;
                                        [b, g, r, a]
                                    };
                                    pixels.extend_from_slice(&[r, g, b, a]);
                                }
                            }
                        })
                        .ok()?;
                    Some(Capture {
                        pixels: Arc::new(pixels),
                        width,
                        height,
                    })
                });
            notify(&notices, Notice::Capture(capture));
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_do_motor_voltam_como_fluent_app() {
        assert_eq!(
            from_engine(&to_engine("fluent://app/a.html")),
            "fluent://app/a.html"
        );
        assert_eq!(to_engine("https://x.com/"), "https://x.com/");
        assert_eq!(from_engine("https://x.com/"), "https://x.com/");
        assert_eq!(percent_decode("css/meu%20estilo.css"), "css/meu estilo.css");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
