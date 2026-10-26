//! WebView do Tomni UI (M18): o widget [`WebView`], uma página da web dentro da tela no
//! controle do sistema (WebView2, WKWebView ou WebKitGTK, pelo `wry`), e o
//! [`WebViewController`], com o estado da página e os comandos do app.
//!
//! Sem a feature `native` (ou sem janela, nos testes e nas ferramentas), o motor é o falso
//! ([`Fake`]).

mod backend;
mod controller;
mod fake;
#[cfg(feature = "native")]
mod native;
mod widget;

pub use backend::{APP_ORIGIN, Capture};
pub use controller::{LoadState, WebViewController};
pub use fake::Fake;
pub use widget::{CAPTURE_WAIT, Loaded, WEBVIEW_MIN_SIZE, WebView, WebViewVisual};
