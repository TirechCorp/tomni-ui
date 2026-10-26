//! O que um widget usa do mundo de fora ao tratar um evento: medidas de texto, clipboard,
//! posição do ponteiro e modificadores.

use std::path::PathBuf;
use std::rc::Rc;

use kurbo::{Point, Rect};

use crate::density::Density;
use crate::event::Modifiers;
use crate::info_bar::InfoBarVisual;
use crate::rich_text::RichTextVisual;
use crate::text_editor::{EditorPoints, EditorText};
use crate::title_bar::WindowRequest;
use crate::widget::Insets;

/// Medidas de texto do `Edit`. O core não mede texto: quem implementa é o `Drawer`, com a
/// mesma fonte que usa para pintar.
pub trait TextMetrics {
    /// Posições onde o cursor pode parar em `text`, na fonte do `Edit`: `(byte, x)` em ordem
    /// crescente de byte, de `(0, 0.0)` até `(text.len(), largura do texto)`.
    fn carets(&mut self, text: &str) -> Vec<(usize, f64)>;

    /// Área do texto num `Edit` que ocupa `rect`, sem a borda e o padding (o da densidade
    /// `density`). O texto começa no canto esquerdo dela, deslocado pela rolagem, e é cortado
    /// nela.
    fn text_area(&self, rect: Rect, density: Density) -> Rect;

    /// Como [`TextMetrics::carets`], para o `Edit` de várias linhas: `(byte, x, linha)` em ordem
    /// crescente de byte. O fim de uma linha é o byte da quebra e o começo da seguinte, o byte
    /// logo depois dela, com `x` zero. O padrão mede cada linha com `carets`.
    fn caret_points(&mut self, text: &str) -> Vec<(usize, f64, usize)> {
        let mut points = Vec::new();
        let mut start = 0;
        for (line, text_line) in text.split('\n').enumerate() {
            for (byte, x) in self.carets(text_line) {
                points.push((start + byte, x, line));
            }
            start += text_line.len() + 1;
        }
        points
    }

    /// Altura de uma linha do texto do `Edit`.
    fn line_height(&self) -> f64 {
        20.0
    }

    /// A área do texto do `Edit` de várias linhas: como [`TextMetrics::text_area`], mas com o
    /// texto no alto do campo, e não no meio.
    fn multiline_area(&self, rect: Rect, density: Density) -> Rect {
        self.text_area(rect, density)
    }

    /// Os pontos do cursor de um [`TextEditor`](crate::TextEditor), com a quebra de linha em
    /// `text.wrap`. O padrão mede cada parágrafo com [`TextMetrics::carets`] e quebra por
    /// caractere, sem olhar as faixas de estilo nem a fonte monoespaçada.
    fn editor_points(&mut self, text: &EditorText) -> Rc<EditorPoints> {
        let mut points = Vec::new();
        let mut width = 0.0_f64;
        let mut line = 0;
        let mut start = 0;
        let paragraphs: Vec<&str> = text.text.split('\n').collect();
        let last = paragraphs.len() - 1;
        for (index, paragraph) in paragraphs.iter().enumerate() {
            let carets = self.carets(paragraph);
            let mut base = 0.0;
            for (i, &(byte, x)) in carets.iter().enumerate() {
                let mut x = x - base;
                if let Some(limit) = text.wrap
                    && i > 0
                    && x > limit
                    && carets[i - 1].1 - base > 0.0
                {
                    // Quebra antes deste caractere: o ponto anterior fecha a linha e abre a
                    // seguinte (mesmo byte, `x` zero).
                    let (prev_byte, prev_x) = carets[i - 1];
                    width = width.max(prev_x - base);
                    line += 1;
                    base = prev_x;
                    points.push((start + prev_byte, 0.0, line));
                    x = carets[i].1 - base;
                }
                width = width.max(x);
                points.push((start + byte, x, line));
            }
            if index < last {
                line += 1;
            }
            start += paragraph.len() + 1;
        }
        Rc::new(EditorPoints {
            points,
            width,
            lines: line + 1,
        })
    }

    /// A área do texto do [`TextEditor`](crate::TextEditor) que ocupa `rect`. O padrão é a
    /// de [`TextMetrics::multiline_area`].
    fn editor_area(&self, rect: Rect, density: Density) -> Rect {
        self.multiline_area(rect, density)
    }

    /// Onde ficam os links de um `RichText` que quebra em `width` (`None`: uma linha só): os
    /// retângulos de cada link, um por linha visual, relativos ao canto do widget. O padrão não
    /// mede texto e não acha link nenhum.
    fn rich_links(&mut self, visual: &RichTextVisual, width: Option<f64>) -> Vec<Vec<Rect>> {
        let _ = (visual, width);
        Vec::new()
    }

    /// Onde fica o botão de ação de uma `InfoBar` com `width` de largura, relativo ao canto da
    /// barra. O padrão não mede texto e não acha botão nenhum.
    fn info_bar_action(&mut self, visual: &InfoBarVisual, width: f64) -> Option<Rect> {
        let _ = (visual, width);
        None
    }
}

/// Medidas fixas: cada `char` tem `advance` px e a área do texto é o retângulo menos
/// `padding`. É o padrão da [`Tree`](crate::Tree) até o `Ui` instalar as do `Drawer`, e serve
/// para testar widgets sem fonte.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedMetrics {
    /// Largura de cada caractere.
    pub advance: f64,
    /// Espaço entre a borda do widget e o texto.
    pub padding: Insets,
}

impl Default for FixedMetrics {
    fn default() -> Self {
        Self {
            advance: 10.0,
            padding: Insets::ZERO,
        }
    }
}

impl TextMetrics for FixedMetrics {
    fn carets(&mut self, text: &str) -> Vec<(usize, f64)> {
        let mut x = 0.0;
        let mut carets = Vec::with_capacity(text.len() + 1);
        for (byte, _) in text.char_indices() {
            carets.push((byte, x));
            x += self.advance;
        }
        carets.push((text.len(), x));
        carets
    }

    fn text_area(&self, rect: Rect, _: Density) -> Rect {
        let p = self.padding;
        Rect::new(
            rect.x0 + p.left,
            rect.y0 + p.top,
            rect.x1 - p.right,
            rect.y1 - p.bottom,
        )
    }
}

/// Uma imagem na área de transferência: pixels RGBA de 8 bits, linha após linha, sem
/// preenchimento (`rgba.len() == width * height * 4`).
#[derive(Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    /// Largura, em pixels.
    pub width: u32,
    /// Altura, em pixels.
    pub height: u32,
    /// Os pixels, em RGBA (alfa direto, não pré-multiplicado).
    pub rgba: Vec<u8>,
}

impl ClipboardImage {
    /// Codifica a imagem em PNG. Dá `Err` se `rgba` não tem o tamanho de `width * height * 4`
    /// ou se a imagem é vazia.
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        let esperado = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(4));
        if self.width == 0 || self.height == 0 {
            return Err("a imagem está vazia".into());
        }
        if esperado != Some(self.rgba.len()) {
            return Err(format!(
                "os pixels não batem com {}×{}: {} bytes",
                self.width,
                self.height,
                self.rgba.len()
            ));
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&self.rgba)
            .map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
        Ok(out)
    }
}

impl std::fmt::Debug for ClipboardImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Área de transferência. A plataforma (M4) liga a do sistema; até lá, e no `TestApp`, vale a
/// [`MemoryClipboard`]. Arquivos e imagem têm implementação padrão que não faz nada: só quem
/// sabe ler e gravar essas coisas as sobrescreve.
pub trait Clipboard {
    /// Texto copiado, se houver.
    fn read(&mut self) -> Option<String>;

    /// Copia `text`.
    fn write(&mut self, text: String);

    /// Arquivos copiados (a lista de caminhos do gerenciador de arquivos), se houver.
    fn read_files(&mut self) -> Option<Vec<PathBuf>> {
        None
    }

    /// A imagem copiada, se houver.
    fn read_image(&mut self) -> Option<ClipboardImage> {
        None
    }

    /// Copia a lista de arquivos `files`.
    fn write_files(&mut self, files: Vec<PathBuf>) {
        let _ = files;
    }

    /// Copia a imagem `image`.
    fn write_image(&mut self, image: ClipboardImage) {
        let _ = image;
    }
}

impl<C: Clipboard + ?Sized> Clipboard for Box<C> {
    fn read(&mut self) -> Option<String> {
        (**self).read()
    }

    fn write(&mut self, text: String) {
        (**self).write(text);
    }

    fn read_files(&mut self) -> Option<Vec<PathBuf>> {
        (**self).read_files()
    }

    fn read_image(&mut self) -> Option<ClipboardImage> {
        (**self).read_image()
    }

    fn write_files(&mut self, files: Vec<PathBuf>) {
        (**self).write_files(files);
    }

    fn write_image(&mut self, image: ClipboardImage) {
        (**self).write_image(image);
    }
}

/// Clipboard em memória, só deste app. Guarda uma coisa por vez: cada `write*` apaga o que
/// havia (texto, arquivos ou imagem), como o do sistema.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryClipboard {
    text: Option<String>,
    files: Option<Vec<PathBuf>>,
    image: Option<ClipboardImage>,
}

impl Clipboard for MemoryClipboard {
    fn read(&mut self) -> Option<String> {
        self.text.clone()
    }

    fn write(&mut self, text: String) {
        *self = Self {
            text: Some(text),
            ..Self::default()
        };
    }

    fn read_files(&mut self) -> Option<Vec<PathBuf>> {
        self.files.clone()
    }

    fn read_image(&mut self) -> Option<ClipboardImage> {
        self.image.clone()
    }

    fn write_files(&mut self, files: Vec<PathBuf>) {
        *self = Self {
            files: Some(files),
            ..Self::default()
        };
    }

    fn write_image(&mut self, image: ClipboardImage) {
        *self = Self {
            image: Some(image),
            ..Self::default()
        };
    }
}

/// Para que serve o texto pedido à plataforma: muda o teclado virtual e desliga o IME e o
/// histórico de digitação nas senhas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InputPurpose {
    /// Texto comum.
    #[default]
    Normal,
    /// Senha.
    Password,
}

/// Pedido de entrada de texto do widget com foco: a plataforma liga o IME (e, no Android, o
/// teclado virtual) e põe a janela de candidatos junto de `area`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextInput {
    /// Onde está o cursor, na janela.
    pub area: Rect,
    /// Tipo de texto.
    pub purpose: InputPurpose,
}

/// Para onde um widget pede que o foco vá ([`EventCx::move_focus`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FocusMove {
    /// Widget anterior que aceita foco dentro do pai (ou do ancestral com
    /// [`Widget::focus_scope`](crate::Widget::focus_scope)), sem dar a volta.
    Prev,
    /// Próximo widget que aceita foco dentro do pai (ou do ancestral com `focus_scope`), sem
    /// dar a volta.
    Next,
    /// Primeiro widget que aceita foco dentro do pai (ou do ancestral com `focus_scope`).
    First,
    /// Último widget que aceita foco dentro do pai (ou do ancestral com `focus_scope`).
    Last,
    /// Primeira parada de Tab dentro do próprio widget (a aba escolhida do `TabView`, depois
    /// do Ctrl+Tab).
    Inside,
    /// O ancestral mais próximo que aceita foco (de um item de um grupo do `NavigationView`
    /// para o grupo).
    Parent,
    /// A próxima parada de Tab depois do widget, como o Tab, sem dar a volta (do botão de
    /// busca do `NavigationView` compacto para a caixa de busca que ele mostra).
    Forward,
    /// A parada de Tab anterior ao widget, como o Shift+Tab, sem dar a volta (o Shift+F6 do
    /// `Terminal`).
    Backward,
}

/// Contexto de um evento: onde está o widget e o ponteiro, que modificadores estão
/// pressionados e os serviços de texto e clipboard.
pub struct EventCx<'a> {
    rect: Rect,
    pointer: Point,
    modifiers: Modifiers,
    metrics: &'a mut dyn TextMetrics,
    clipboard: &'a mut dyn Clipboard,
    focus: Option<FocusMove>,
    focus_later: Option<String>,
    window: Option<WindowRequest>,
}

impl<'a> EventCx<'a> {
    /// Contexto para um widget em `rect`. A árvore monta um a cada evento; testes de widget
    /// podem montar à mão.
    pub fn new(
        rect: Rect,
        pointer: Point,
        modifiers: Modifiers,
        metrics: &'a mut dyn TextMetrics,
        clipboard: &'a mut dyn Clipboard,
    ) -> Self {
        Self {
            rect,
            pointer,
            modifiers,
            metrics,
            clipboard,
            focus: None,
            focus_later: None,
            window: None,
        }
    }

    /// Retângulo do widget na janela.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Última posição do ponteiro, na janela.
    pub fn pointer(&self) -> Point {
        self.pointer
    }

    /// Modificadores pressionados.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Medidas de texto.
    pub fn metrics(&mut self) -> &mut dyn TextMetrics {
        &mut *self.metrics
    }

    /// Área de transferência.
    pub fn clipboard(&mut self) -> &mut dyn Clipboard {
        &mut *self.clipboard
    }

    /// Pede que o foco vá para outro widget do grupo, como se viesse do teclado: as setas do
    /// `RadioButtons` e das abas do `TabView`. A árvore move depois que o evento termina; se
    /// não houver para onde ir, o foco fica.
    pub fn move_focus(&mut self, to: FocusMove) {
        self.focus = Some(to);
    }

    /// O pedido de [`EventCx::move_focus`], se houve.
    pub fn focus_request(&self) -> Option<FocusMove> {
        self.focus
    }

    /// Pede o foco para o widget com o id `id`, que pode ainda não existir: a árvore tenta
    /// depois do layout do frame seguinte ao evento (uma linha de lista virtualizada que o
    /// teclado acabou de pedir) e rola até ele. O foco chega como se viesse do teclado. Se o
    /// widget não aparece nessas tentativas, o pedido é descartado.
    pub fn focus_later(&mut self, id: impl Into<String>) {
        self.focus_later = Some(id.into());
    }

    /// O pedido de [`EventCx::focus_later`], se houve, tirando-o.
    pub fn take_focus_later(&mut self) -> Option<String> {
        self.focus_later.take()
    }

    /// Pede algo à janela: arrastar, maximizar, fechar (a `TitleBar`). A árvore junta os
    /// pedidos, e a plataforma atende depois do evento.
    pub fn request_window(&mut self, request: WindowRequest) {
        self.window = Some(request);
    }

    /// O pedido de [`EventCx::request_window`], se houve.
    pub fn window_request(&self) -> Option<WindowRequest> {
        self.window
    }
}

impl std::fmt::Debug for EventCx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventCx")
            .field("rect", &self.rect)
            .field("pointer", &self.pointer)
            .field("modifiers", &self.modifiers)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_png_volta_nos_mesmos_pixels() {
        let rgba = vec![
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 10, 20, 30, 128,
        ];
        let imagem = ClipboardImage {
            width: 2,
            height: 2,
            rgba: rgba.clone(),
        };
        let png = imagem.to_png().unwrap();
        let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (2, 2));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(&buf[..info.buffer_size()], &rgba[..]);
    }

    #[test]
    fn to_png_com_pixels_do_tamanho_errado_da_erro() {
        let curta = ClipboardImage {
            width: 2,
            height: 2,
            rgba: vec![0; 15],
        };
        assert!(curta.to_png().is_err());
        let vazia = ClipboardImage {
            width: 0,
            height: 3,
            rgba: Vec::new(),
        };
        assert!(vazia.to_png().is_err());
    }

    #[test]
    fn cada_write_limpa_os_outros_dois() {
        let imagem = ClipboardImage {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 4],
        };
        let arquivos = vec![PathBuf::from("/tmp/a.txt")];
        let mut c = MemoryClipboard::default();
        c.write("texto".into());
        assert_eq!(c.read().as_deref(), Some("texto"));
        c.write_files(arquivos.clone());
        assert_eq!((c.read(), c.read_image()), (None, None));
        assert_eq!(c.read_files(), Some(arquivos));
        c.write_image(imagem.clone());
        assert_eq!((c.read(), c.read_files()), (None, None));
        assert_eq!(c.read_image(), Some(imagem));
        c.write("de novo".into());
        assert_eq!((c.read_files(), c.read_image()), (None, None));
        assert_eq!(c.read().as_deref(), Some("de novo"));
    }
}
