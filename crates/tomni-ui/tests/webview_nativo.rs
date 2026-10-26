//! O `WebView` com o motor do sistema (WebKitGTK, WebView2, WKWebView): abre uma janela de
//! verdade, carrega uma página embutida com um script em `.asset`, troca mensagens com ela e
//! roda um `eval`. Sem tela (sem `DISPLAY` no Linux), o teste é pulado.
//!
//! `cargo test -p tomni-ui --features webview --test webview_nativo`; no Linux sem monitor,
//! com o `xvfb-run`. Roda fora do harness do libtest porque a janela e o GTK precisam da
//! thread principal.

use std::time::Duration;

use tomni_ui::prelude::*;

/// Quanto o teste espera a página antes de desistir.
const LIMITE: Duration = Duration::from_secs(60);

const PAGINA: &str = r#"<!doctype html>
<html lang="pt-BR">
<head><meta charset="utf-8"><title>Nativo</title><script src="app.js"></script></head>
<body><p>Teste do motor do sistema.</p></body>
</html>
"#;

const SCRIPT: &str = r#"
window.fluent.addEventListener('message', (e) => window.fluent.postMessage('eco:' + e.data));
window.addEventListener('load', () => window.fluent.postMessage('pronto:' + document.title));
"#;

fn falha(motivo: &str) -> ! {
    eprintln!("webview_nativo: falhou: {motivo}");
    std::process::exit(1);
}

fn tela() -> impl View {
    let web = WebViewController::new();
    Form::new("WebView nativo").child(
        WebView::html(PAGINA)
            .asset("app.js", SCRIPT)
            .id("pagina")
            .grow(1)
            .controller(web)
            .on_load(|carga| {
                if let Some(erro) = &carga.error {
                    falha(&format!("a página não carregou ({}): {erro}", carga.url));
                }
            })
            .on_message(move |texto| match texto.as_str() {
                "pronto:Nativo" => {
                    if web.url() != format!("{}/index.html", tomni_ui::webview::APP_ORIGIN) {
                        falha(&format!("endereço inesperado: {}", web.url()));
                    }
                    web.post_message("ping");
                }
                "eco:ping" => web.eval("window.fluent.postMessage('eval:' + (1 + 1))"),
                "eval:2" => {
                    if web.title() != "Nativo" {
                        falha(&format!("título inesperado: {:?}", web.title()));
                    }
                    println!("webview_nativo: ok");
                    std::process::exit(0);
                }
                outro => falha(&format!("mensagem inesperada: {outro:?}")),
            }),
    )
}

fn main() {
    if cfg!(target_os = "linux")
        && std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
    {
        println!("webview_nativo: pulado (sem tela)");
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(LIMITE);
        falha("a página não respondeu a tempo");
    });
    if let Err(erro) = tomni_ui::App::new(tela).size(640, 480).run() {
        falha(&format!("a janela não abriu: {erro}"));
    }
    falha("a janela fechou antes do fim");
}
