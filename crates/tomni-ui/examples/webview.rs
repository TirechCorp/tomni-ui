//! `WebView`: uma página da web dentro da tela, no motor do sistema (WebView2 no Windows,
//! WKWebView no macOS, WebKitGTK no Linux). `cargo run -p tomni-ui --example webview
//! --features webview`; só com `webview-widget`, a página é o quadro do motor falso.
//!
//! As páginas aqui são embutidas no app (`WebView::html` e `.asset`), servidas em
//! `fluent://app/`. A barra de cima usa o `WebViewController`: voltar, avançar, recarregar e
//! o título da página. A página e o app conversam pelo `window.fluent`: o botão da página
//! manda uma mensagem (`on_message`) e o "Mandar a hora" responde (`post_message`).
//!
//! O F6 leva o foco da página de volta ao app. Links para outros sites abrem no navegador.

use tomni_ui::prelude::*;

const INICIO: &str = r#"<!doctype html>
<html lang="pt-BR">
<head>
<meta charset="utf-8">
<title>Início</title>
<style>
  :root { color-scheme: light dark; font-family: system-ui, sans-serif; }
  body { margin: 24px; line-height: 1.5; }
  button { font: inherit; padding: 6px 12px; }
</style>
</head>
<body>
  <h1>Olá do WebView</h1>
  <p>Esta página vem de dentro do app, em <code>fluent://app/</code>.</p>
  <p><button id="ola">Mandar uma mensagem ao app</button></p>
  <p id="resposta">O app ainda não respondeu.</p>
  <p><a href="sobre.html">Sobre</a> · <a href="https://www.rust-lang.org/">rust-lang.org</a></p>
  <script>
    document.getElementById('ola').onclick = () => window.fluent.postMessage('olá, app');
    window.fluent.addEventListener('message', (e) => {
      document.getElementById('resposta').textContent = 'O app disse: ' + e.data;
    });
  </script>
</body>
</html>
"#;

const SOBRE: &str = r#"<!doctype html>
<html lang="pt-BR">
<head><meta charset="utf-8"><title>Sobre</title>
<style>:root { color-scheme: light dark; font-family: system-ui, sans-serif; } body { margin: 24px; }</style>
</head>
<body><h1>Sobre</h1><p>A segunda página, para o histórico. Use o voltar da barra.</p></body>
</html>
"#;

/// Um navegador pequeno: a barra com o histórico e o título, a página e as mensagens.
pub fn navegador() -> impl View {
    let web = WebViewController::new();
    let recebida = signal(String::from("Nenhuma mensagem da página."));
    Form::new("WebView").child(
        Column::new()
            .gap(8)
            .padding(16)
            .child(
                Row::new()
                    .gap(4)
                    .align(Align::Center)
                    .child(
                        Button::icon_only(Symbol::ArrowLeft)
                            .id("voltar")
                            .label("Voltar")
                            .disabled(move || !web.can_go_back())
                            .on_click(move || web.go_back()),
                    )
                    .child(
                        Button::icon_only(Symbol::ArrowRight)
                            .id("avancar")
                            .label("Avançar")
                            .disabled(move || !web.can_go_forward())
                            .on_click(move || web.go_forward()),
                    )
                    .child(
                        Button::icon_only(Symbol::ArrowClockwise)
                            .id("recarregar")
                            .label("Recarregar")
                            .on_click(move || web.reload()),
                    )
                    .child(
                        Text::new(move || match web.state() {
                            LoadState::Loading => String::from("Carregando…"),
                            LoadState::Ready => web.title(),
                            LoadState::Failed => String::from("Erro"),
                        })
                        .id("titulo")
                        .max_lines(1),
                    ),
            )
            .child(
                WebView::html(INICIO)
                    .asset("sobre.html", SOBRE)
                    .id("pagina")
                    .grow(1)
                    .controller(web)
                    .on_message(move |texto| recebida.set(format!("A página disse: {texto}"))),
            )
            .child(
                Row::new()
                    .gap(8)
                    .align(Align::Center)
                    .child(Text::new(recebida).id("recebida"))
                    .child(
                        Button::new("Mandar a hora")
                            .id("mandar")
                            .height(44)
                            .on_click(move || web.post_message(hora())),
                    ),
            ),
    )
}

/// A hora UTC, `hh:mm:ss`.
fn hora() -> String {
    let segundos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (h, m, s) = (segundos / 3600 % 24, segundos / 60 % 60, segundos % 60);
    format!("{h:02}:{m:02}:{s:02} UTC")
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(navegador).size(900, 640).run()
}
