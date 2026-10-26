//! O motor falso: sem janela (nos testes, no lint, nas ferramentas), o `WebView` carrega as
//! páginas de mentira e registra o que o app pediu.

use std::any::Any;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use tomni_ui_core::kurbo::Rect;

use crate::backend::{
    Assets, Backend, Capture, Decision, Notice, Notices, Policy, app_path, notify, origin,
};

/// Motor falso do [`WebView`](crate::WebView), usado quando não há janela: carrega as páginas
/// sem rede (a carga termina no `poll` seguinte), guarda o que o app pediu e deixa o teste
/// fazer o papel da página.
///
/// O `TestApp` chega a ele pelos métodos `webview_*`.
pub struct Fake {
    notices: Notices,
    policy: Rc<Policy>,
    assets: Assets,
    pending: VecDeque<Notice>,
    entries: Vec<String>,
    index: usize,
    navigations: Vec<String>,
    scripts: Vec<String>,
    posted: Vec<String>,
    opened: Vec<String>,
    rect: Option<Rect>,
    dark: bool,
    focused: bool,
    held: bool,
}

impl Fake {
    pub(crate) fn new(notices: Notices, policy: Rc<Policy>, assets: Assets) -> Self {
        Self {
            notices,
            policy,
            assets,
            pending: VecDeque::new(),
            entries: Vec::new(),
            index: 0,
            navigations: Vec::new(),
            scripts: Vec::new(),
            posted: Vec::new(),
            opened: Vec::new(),
            rect: None,
            dark: false,
            focused: false,
            held: false,
        }
    }

    /// As URLs que o app mandou carregar, na ordem (a inicial primeiro).
    pub fn navigations(&self) -> &[String] {
        &self.navigations
    }

    /// Os scripts que o app rodou na página (`WebViewController::eval`).
    pub fn scripts(&self) -> &[String] {
        &self.scripts
    }

    /// As mensagens que o app mandou à página (`WebViewController::post_message`).
    pub fn posted(&self) -> &[String] {
        &self.posted
    }

    /// Os endereços que a página abriu fora, no navegador do sistema.
    pub fn opened(&self) -> &[String] {
        &self.opened
    }

    /// A página atual.
    pub fn url(&self) -> Option<&str> {
        self.entries.get(self.index).map(String::as_str)
    }

    /// Onde o controle estaria na janela, ou `None` se estaria escondido.
    pub fn rect(&self) -> Option<Rect> {
        self.rect
    }

    /// Se a página está no tema escuro.
    pub fn dark(&self) -> bool {
        self.dark
    }

    /// Se o controle tem o foco do sistema.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// A página manda `text` ao app (`window.fluent.postMessage(text)`).
    pub fn page_message(&mut self, text: impl Into<String>) {
        notify(&self.notices, Notice::Message(text.into()));
    }

    /// A página navega para `url` (um link clicado): carrega se a origem é permitida, abre
    /// fora se é `http`, `https`, `mailto` ou `tel`, ou ignora. Devolve a decisão: `true` se
    /// carregou.
    pub fn follow(&mut self, url: &str) -> bool {
        match self.policy.decide(url) {
            Decision::Allow => {
                self.load(url.to_owned(), true);
                true
            }
            Decision::Open => {
                self.opened.push(url.to_owned());
                false
            }
            Decision::Block => false,
        }
    }

    /// A carga em andamento (ou a última) falha com `error`.
    pub fn fail(&mut self, error: impl Into<String>) {
        self.pending.retain(|n| !matches!(n, Notice::Finished(_)));
        let url = self.url().unwrap_or_default().to_owned();
        self.pending.push_back(Notice::Failed {
            url,
            error: error.into(),
        });
        tomni_ui_core::wake();
    }

    /// A página ganha o foco (um clique nela).
    pub fn page_focus(&mut self) {
        self.focused = true;
        notify(&self.notices, Notice::Focus);
    }

    /// Segura as respostas da página (uma rede lenta): com `hold`, as cargas ficam em
    /// andamento até soltar.
    pub fn hold(&mut self, hold: bool) {
        self.held = hold;
        tomni_ui_core::wake();
    }

    /// F6 (Shift+F6 com `back`) dentro da página.
    pub fn page_f6(&mut self, back: bool) {
        notify(&self.notices, Notice::Leave { back });
    }

    fn load(&mut self, url: String, push: bool) {
        if push {
            self.entries.truncate(self.index + 1);
            self.entries.push(url.clone());
            self.index = self.entries.len() - 1;
        }
        self.pending.retain(|n| !matches!(n, Notice::Finished(_)));
        notify(&self.notices, Notice::Started(url.clone()));
        let title = app_path(&url)
            .and_then(|path| self.assets.get(path))
            .and_then(|asset| title_of(&asset.bytes))
            .or_else(|| {
                origin(&url).map(|o| o.split_once("://").map_or(o.clone(), |(_, h)| h.to_owned()))
            })
            .unwrap_or_default();
        self.pending.push_back(Notice::Finished(url));
        self.pending.push_back(Notice::Title(title));
        self.pending.push_back(Notice::History);
    }
}

/// O texto do `<title>` de um HTML.
fn title_of(html: &[u8]) -> Option<String> {
    let html = std::str::from_utf8(html).ok()?;
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let start = start + lower[start..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    Some(html[start..end].trim().to_owned())
}

impl Backend for Fake {
    fn navigate(&mut self, url: &str) {
        self.navigations.push(url.to_owned());
        self.load(url.to_owned(), true);
    }

    fn reload(&mut self) {
        if let Some(url) = self.url().map(str::to_owned) {
            self.load(url, false);
        }
    }

    fn back(&mut self) {
        if self.index > 0 {
            self.index -= 1;
            self.load(self.entries[self.index].clone(), false);
        }
    }

    fn forward(&mut self) {
        if self.index + 1 < self.entries.len() {
            self.index += 1;
            self.load(self.entries[self.index].clone(), false);
        }
    }

    fn eval(&mut self, script: &str) {
        self.scripts.push(script.to_owned());
    }

    fn post(&mut self, text: &str) {
        self.posted.push(text.to_owned());
    }

    fn history(&self) -> (bool, bool) {
        (self.index > 0, self.index + 1 < self.entries.len())
    }

    fn place(&mut self, rect: Option<Rect>) {
        self.rect = rect;
    }

    fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
    }

    fn focus(&mut self) {
        self.focused = true;
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }

    fn capture(&mut self) {
        let capture = self.rect.map(|rect| fake_capture(rect, self.dark));
        self.pending.push_back(Notice::Capture(capture));
    }

    fn turn(&mut self) {
        if self.held {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        self.notices.borrow_mut().extend(pending);
    }

    fn busy(&self) -> bool {
        !self.held && !self.pending.is_empty()
    }

    fn native(&self) -> bool {
        false
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Uma página de mentira do tamanho de `rect`: o fundo (claro ou escuro) com barras no lugar
/// do título e das linhas de texto.
fn fake_capture(rect: Rect, dark: bool) -> Capture {
    let width = rect.width().round().max(1.0) as u32;
    let height = rect.height().round().max(1.0) as u32;
    let (page, ink) = if dark { (32, 80) } else { (255, 214) };
    let bar = |x: u32, y: u32| {
        let (left, right) = (24, width.saturating_sub(24));
        if x < left || x >= right {
            return false;
        }
        if (24..44).contains(&y) {
            return x < left + (right - left) * 3 / 5;
        }
        y >= 68 && y + 24 < height && (y - 68) % 24 < 10
    };
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let v = if bar(x, y) { ink } else { page };
            pixels.extend_from_slice(&[v, v, v, 255]);
        }
    }
    Capture {
        pixels: Arc::new(pixels),
        width,
        height,
    }
}

/// Motor de quando o do sistema não abriu (sem WebKitGTK, sem WebView2): toda carga falha com
/// o erro dele.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
pub(crate) struct Unavailable {
    pub(crate) notices: Notices,
    pub(crate) error: String,
}

impl Backend for Unavailable {
    fn navigate(&mut self, url: &str) {
        notify(
            &self.notices,
            Notice::Failed {
                url: url.to_owned(),
                error: self.error.clone(),
            },
        );
    }

    fn reload(&mut self) {}

    fn back(&mut self) {}

    fn forward(&mut self) {}

    fn eval(&mut self, _: &str) {}

    fn history(&self) -> (bool, bool) {
        (false, false)
    }

    fn place(&mut self, _: Option<Rect>) {}

    fn set_dark(&mut self, _: bool) {}

    fn focus(&mut self) {}

    fn unfocus(&mut self) {}

    fn capture(&mut self) {
        notify(&self.notices, Notice::Capture(None));
    }

    fn native(&self) -> bool {
        false
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titulo_do_html() {
        assert_eq!(
            title_of(b"<html><head><TITLE lang=pt> Ajuda </title>").as_deref(),
            Some("Ajuda")
        );
        assert_eq!(title_of(b"<p>sem</p>"), None);
    }

    #[test]
    fn captura_falsa_tem_o_tamanho_do_retangulo() {
        let capture = fake_capture(Rect::new(0.0, 0.0, 100.0, 80.0), true);
        assert_eq!((capture.width, capture.height), (100, 80));
        assert_eq!(capture.pixels.len(), 100 * 80 * 4);
        assert_eq!(&capture.pixels[..4], &[32, 32, 32, 255]);
    }
}
