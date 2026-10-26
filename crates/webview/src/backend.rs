//! O motor por trás do widget: o do sistema ([`crate::native`], com a feature `native`) ou o
//! falso ([`Fake`](crate::Fake)), e o que os dois avisam ao widget.

use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use tomni_ui_core::kurbo::Rect;

/// Origem das páginas embutidas com [`WebView::html`](crate::WebView::html) e
/// [`WebView::asset`](crate::WebView::asset).
pub const APP_ORIGIN: &str = "fluent://app";

/// Imagem da página, tirada para ficar no lugar dela enquanto algo a cobre.
#[derive(Clone)]
pub struct Capture {
    /// Pixels RGBA com alfa pré-multiplicado, linha a linha.
    pub pixels: Arc<Vec<u8>>,
    /// Largura, em pixels físicos.
    pub width: u32,
    /// Altura, em pixels físicos.
    pub height: u32,
}

impl PartialEq for Capture {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.pixels, &other.pixels)
    }
}

impl fmt::Debug for Capture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Capture")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// O que o motor avisa ao widget. Os avisos esperam numa fila ([`Notices`]) até o próximo
/// `poll`, na thread da interface.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Notice {
    /// Uma página começou a carregar.
    Started(String),
    /// A página terminou de carregar.
    Finished(String),
    /// A página não carregou (sem rede, endereço errado).
    Failed { url: String, error: String },
    /// O `<title>` mudou.
    Title(String),
    /// A página mandou uma mensagem (`window.fluent.postMessage`).
    Message(String),
    /// O histórico pode ter mudado (`pushState`, voltar): o widget pergunta ao motor.
    History,
    /// A página ganhou o foco sozinha (um clique nela).
    Focus,
    /// O F6 dentro da página: o foco sai, para trás com o Shift.
    Leave { back: bool },
    /// A página pediu uma janela nova para uma origem permitida: abre no próprio WebView.
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    Open(String),
    /// A captura pedida, ou `None` se o motor não captura.
    Capture(Option<Capture>),
}

/// Fila dos avisos do motor, compartilhada com os handlers dele.
pub(crate) type Notices = Rc<RefCell<VecDeque<Notice>>>;

/// Põe `notice` na fila e acorda o loop de frame.
pub(crate) fn notify(notices: &Notices, notice: Notice) {
    notices.borrow_mut().push_back(notice);
    tomni_ui_core::wake();
}

/// O motor: o controle do sistema ou o falso.
pub(crate) trait Backend {
    /// Carrega `url` (o widget já normalizou para `fluent://app/...`).
    fn navigate(&mut self, url: &str);
    /// Carrega a página de novo.
    fn reload(&mut self);
    /// Volta uma página no histórico.
    fn back(&mut self);
    /// Avança uma página no histórico.
    fn forward(&mut self);
    /// Roda `script` na página.
    fn eval(&mut self, script: &str);
    /// Entrega `text` à página, no `window.fluent`.
    fn post(&mut self, text: &str) {
        self.eval(&post_script(text));
    }
    /// `(pode voltar, pode avançar)`.
    fn history(&self) -> (bool, bool);
    /// Mostra o controle em `rect` (pixels lógicos) ou o esconde (`None`).
    fn place(&mut self, rect: Option<Rect>);
    /// Tema escuro (`prefers-color-scheme: dark`).
    fn set_dark(&mut self, dark: bool);
    /// Passa o foco do sistema ao controle.
    fn focus(&mut self);
    /// Devolve o foco do sistema à janela.
    fn unfocus(&mut self);
    /// Pede uma captura da página; ela chega como [`Notice::Capture`].
    fn capture(&mut self);
    /// Avança o que o motor deixou para depois (o falso conclui as cargas aqui).
    fn turn(&mut self) {}
    /// Se [`Backend::turn`] tem algo a fazer.
    fn busy(&self) -> bool {
        false
    }
    /// Se é o controle do sistema, numa janela filha (e não o falso).
    fn native(&self) -> bool;
    /// Para o `TestApp` chegar ao motor falso.
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Para o `TestApp` chegar ao motor falso.
    fn as_any(&self) -> &dyn Any;
}

/// Um arquivo servido em `fluent://app/`.
#[derive(Clone, Debug)]
pub(crate) struct Asset {
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    pub(crate) mime: &'static str,
    pub(crate) bytes: Arc<[u8]>,
}

/// Os arquivos de `fluent://app/`, pelo caminho sem a barra inicial (`index.html`).
pub(crate) type Assets = Arc<HashMap<String, Asset>>;

/// Tipo do arquivo pela extensão, para o `Content-Type`.
pub(crate) fn mime(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map_or("", |(_, ext)| ext);
    match ext.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// Caminho de `url` dentro de `fluent://app/`, sem a barra inicial, a consulta e o `#`; `None`
/// se a URL é de outra origem.
pub(crate) fn app_path(url: &str) -> Option<&str> {
    let rest = url.strip_prefix(APP_ORIGIN)?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    let path = &rest[..end];
    Some(if path.is_empty() { "index.html" } else { path })
}

/// Origem (`esquema://host[:porta]`) de `url`, em minúsculas; `None` se não tem host.
pub(crate) fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    (!scheme.is_empty() && !host.is_empty())
        .then(|| format!("{scheme}://{host}").to_ascii_lowercase())
}

/// O que fazer com uma navegação pedida pela página (um link, um `location.href`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Carrega no WebView.
    Allow,
    /// Abre no navegador (ou no programa) do sistema e fica onde está.
    Open,
    /// Ignora.
    Block,
}

/// As origens em que a página pode navegar: `fluent://app`, a da URL inicial, as de
/// [`WebView::allow`](crate::WebView::allow) e as das URLs que o app carregou.
#[derive(Debug, Default)]
pub(crate) struct Policy {
    origins: RefCell<Vec<String>>,
}

impl Policy {
    /// Permite a origem de `url`.
    pub(crate) fn allow(&self, url: &str) {
        if let Some(origin) = origin(url) {
            let mut origins = self.origins.borrow_mut();
            if !origins.contains(&origin) {
                origins.push(origin);
            }
        }
    }

    /// Decide a navegação para `url`.
    pub(crate) fn decide(&self, url: &str) -> Decision {
        let scheme = url
            .split_once(':')
            .map_or("", |(scheme, _)| scheme)
            .to_ascii_lowercase();
        if app_path(url).is_some() || matches!(scheme.as_str(), "about" | "data" | "blob") {
            return Decision::Allow;
        }
        if origin(url).is_some_and(|origin| self.origins.borrow().contains(&origin)) {
            return Decision::Allow;
        }
        match scheme.as_str() {
            "http" | "https" | "mailto" | "tel" => Decision::Open,
            _ => Decision::Block,
        }
    }
}

/// Prefixo das mensagens do script de início para o widget; as da página começam com `m`.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
pub(crate) const INTERNAL: &str = "fluent:";

/// Script de todas as páginas: o F6 devolve o foco ao app, os avisos de histórico e de foco e
/// o `window.fluent`, por onde a página e o app conversam.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
pub(crate) const INIT_SCRIPT: &str = r#"(() => {
  if (window.fluent) return;
  const ipc = (m) => window.ipc.postMessage(m);
  document.addEventListener('keydown', (e) => {
    if (e.key === 'F6') {
      e.preventDefault();
      ipc(e.shiftKey ? 'fluent:f6-back' : 'fluent:f6');
    }
  }, true);
  const hist = () => ipc('fluent:hist');
  window.addEventListener('pageshow', hist);
  window.addEventListener('popstate', hist);
  window.addEventListener('hashchange', hist);
  if (window.navigation) navigation.addEventListener('currententrychange', hist);
  window.addEventListener('focus', () => ipc('fluent:focus'));
  const fluent = new EventTarget();
  fluent.postMessage = (data) => ipc('m' + String(data));
  window.fluent = fluent;
})();"#;

/// O aviso que corresponde a uma mensagem `body` do script de início ou da página.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
pub(crate) fn ipc_notice(body: &str) -> Option<Notice> {
    if let Some(text) = body.strip_prefix('m') {
        return Some(Notice::Message(text.to_owned()));
    }
    match body.strip_prefix(INTERNAL)? {
        "f6" => Some(Notice::Leave { back: false }),
        "f6-back" => Some(Notice::Leave { back: true }),
        "hist" => Some(Notice::History),
        "focus" => Some(Notice::Focus),
        _ => None,
    }
}

/// `text` como literal de string do JavaScript.
pub(crate) fn js_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Fecharia um `<script>` se o texto parar dentro de um.
            '<' => out.push_str("\\u003c"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// O script que entrega `text` à página, como um `MessageEvent` no `window.fluent`.
pub(crate) fn post_script(text: &str) -> String {
    format!(
        "window.fluent && window.fluent.dispatchEvent(new MessageEvent('message', {{ data: {} }}));",
        js_string(text)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caminhos_e_origens() {
        assert_eq!(app_path("fluent://app/index.html"), Some("index.html"));
        assert_eq!(app_path("fluent://app/"), Some("index.html"));
        assert_eq!(app_path("fluent://app"), Some("index.html"));
        assert_eq!(app_path("fluent://app/css/a.css?v=2#x"), Some("css/a.css"));
        assert_eq!(app_path("https://app/x"), None);
        assert_eq!(
            origin("https://user@Exemplo.com:8080/a?b").as_deref(),
            Some("https://exemplo.com:8080")
        );
        assert_eq!(origin("mailto:ana@x.com"), None);
        assert_eq!(mime("a/B.HTML"), "text/html; charset=utf-8");
        assert_eq!(mime("sem-extensao"), "application/octet-stream");
    }

    #[test]
    fn politica_de_navegacao() {
        let policy = Policy::default();
        policy.allow("https://docs.exemplo.com/inicio");
        assert_eq!(policy.decide("fluent://app/b.html"), Decision::Allow);
        assert_eq!(policy.decide("about:blank"), Decision::Allow);
        assert_eq!(
            policy.decide("https://docs.exemplo.com/outra"),
            Decision::Allow
        );
        assert_eq!(policy.decide("https://outro.com/"), Decision::Open);
        assert_eq!(policy.decide("mailto:ana@x.com"), Decision::Open);
        assert_eq!(policy.decide("file:///etc/passwd"), Decision::Block);
        assert_eq!(policy.decide("javascript:alert(1)"), Decision::Block);
    }

    #[test]
    fn mensagens_do_script() {
        assert_eq!(ipc_notice("molá"), Some(Notice::Message("olá".into())));
        assert_eq!(ipc_notice("m"), Some(Notice::Message(String::new())));
        assert_eq!(ipc_notice("fluent:f6"), Some(Notice::Leave { back: false }));
        assert_eq!(
            ipc_notice("fluent:f6-back"),
            Some(Notice::Leave { back: true })
        );
        assert_eq!(ipc_notice("outra"), None);
        assert_eq!(
            js_string("a\"b\\c\n</script>\u{2028}"),
            r#""a\"b\\c\n\u003c/script>\u2028""#
        );
    }
}
