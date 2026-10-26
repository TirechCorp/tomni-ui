//! `Image`: uma imagem PNG, JPEG, GIF, WebP ou SVG, dos bytes do programa ou de um arquivo.

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::widget::{Common, Look, Widget, common_builders};

/// Como a imagem ocupa o retângulo dela, como o `Stretch` do WinUI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Stretch {
    /// No tamanho natural, centralizada; o que passa do retângulo é cortado.
    None,
    /// Esticada para o retângulo, sem manter a proporção.
    Fill,
    /// Inteira dentro do retângulo, na proporção dela, centralizada (o padrão, como no WinUI).
    #[default]
    Uniform,
    /// Cobre o retângulo inteiro, na proporção dela, centralizada; o que sobra é cortado.
    UniformToFill,
}

impl Stretch {
    /// Nome em minúsculas (`uniform_to_fill`), usado na árvore do `tomni-ui tree`.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Fill => "fill",
            Self::Uniform => "uniform",
            Self::UniformToFill => "uniform_to_fill",
        }
    }
}

/// Os bytes de uma imagem, ainda não decodificados. Vem de `&'static [u8]` (o
/// `include_bytes!`), de `Vec<u8>` ou de `Arc<[u8]>`.
#[derive(Clone)]
pub struct ImageSource {
    key: SourceKey,
    bytes: Result<Bytes, String>,
}

/// Identidade dos bytes: o `Drawer` guarda a imagem decodificada por ela.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SourceKey {
    /// Bytes do programa: o endereço e o tamanho, os mesmos em toda `Image` que os usa.
    Static(usize, usize),
    /// Bytes lidos ou passados em tempo de execução: um número novo a cada um.
    Owned(u64),
}

#[derive(Clone)]
enum Bytes {
    Static(&'static [u8]),
    Owned(Arc<[u8]>),
}

impl ImageSource {
    fn owned(bytes: Result<Arc<[u8]>, String>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            key: SourceKey::Owned(NEXT.fetch_add(1, Ordering::Relaxed)),
            bytes: bytes.map(Bytes::Owned),
        }
    }

    /// Os bytes do arquivo `path`, lidos agora. Se a leitura falhar, guarda o erro, que a
    /// `Image` mostra como um quadro vazio e a árvore como `error`.
    pub fn file(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        Self::owned(
            std::fs::read(path)
                .map(Arc::from)
                .map_err(|e| format!("não deu para ler {}: {e}", path.display())),
        )
    }

    /// Os bytes, ou o erro ao ler o arquivo.
    pub fn bytes(&self) -> Result<&[u8], &str> {
        match &self.bytes {
            Ok(Bytes::Static(bytes)) => Ok(bytes),
            Ok(Bytes::Owned(bytes)) => Ok(bytes),
            Err(e) => Err(e),
        }
    }

    /// Número que identifica estes bytes: o mesmo para o mesmo `include_bytes!`, e um novo para
    /// cada arquivo lido ou `Vec` passado. Serve de chave para guardar a imagem decodificada.
    pub fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        self.key.hash(&mut hasher);
        hasher.finish()
    }
}

impl PartialEq for ImageSource {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl fmt::Debug for ImageSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.bytes() {
            Ok(bytes) => write!(f, "ImageSource({} bytes)", bytes.len()),
            Err(e) => write!(f, "ImageSource(erro: {e})"),
        }
    }
}

impl From<&'static [u8]> for ImageSource {
    fn from(bytes: &'static [u8]) -> Self {
        Self {
            key: SourceKey::Static(bytes.as_ptr() as usize, bytes.len()),
            bytes: Ok(Bytes::Static(bytes)),
        }
    }
}

impl<const N: usize> From<&'static [u8; N]> for ImageSource {
    fn from(bytes: &'static [u8; N]) -> Self {
        Self::from(bytes.as_slice())
    }
}

impl From<Vec<u8>> for ImageSource {
    fn from(bytes: Vec<u8>) -> Self {
        Self::owned(Ok(bytes.into()))
    }
}

impl From<Arc<[u8]>> for ImageSource {
    fn from(bytes: Arc<[u8]>) -> Self {
        Self::owned(Ok(bytes))
    }
}

/// Tudo o que o `Drawer` precisa saber da `Image` para medi-la e pintá-la.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageVisual {
    /// Os bytes.
    pub source: ImageSource,
    /// Como ocupa o retângulo.
    pub stretch: Stretch,
    /// Raio dos cantos.
    pub corner_radius: f64,
    /// O tamanho natural que o app informou ([`Image::natural_size`]), em pixels: com ele a
    /// imagem é medida sem decodificar.
    pub natural_size: Option<(u32, u32)>,
}

/// Uma imagem PNG, JPEG, GIF, WebP ou SVG.
///
/// ```
/// use tomni_ui_core::{Image, Stretch};
///
/// // Num app, os bytes costumam vir de `include_bytes!("logo.svg")`.
/// static LOGO: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"/>"#;
/// let logo = Image::bytes(LOGO).id("logo").alt("Logotipo da empresa");
/// let foto = Image::file("fotos/praia.jpg")
///     .width(320)
///     .height(180)
///     .stretch(Stretch::UniformToFill)
///     .corner_radius(8)
///     .alt("Praia ao entardecer");
/// ```
///
/// O formato vem dos bytes, não da extensão. O GIF e o WebP animados mostram o primeiro quadro.
/// O tamanho natural é o da imagem (os pixels do PNG, do JPEG, do GIF e do WebP, o
/// `width`/`height` do SVG); com menos largura, ela encolhe na proporção. Com
/// `.width` e `.height`, o [`Stretch`] diz como ela ocupa o retângulo (o padrão é o
/// [`Stretch::Uniform`]). O SVG é desenhado no tamanho em que aparece, sem borrar.
///
/// Uma imagem que não abre (arquivo que não existe, bytes de outro formato) vira um quadro
/// vazio do mesmo tamanho, sem pânico, e a árvore mostra o motivo em `error`.
///
/// Toda imagem pede um texto alternativo ([`Image::alt`]), que a árvore mostra, ou
/// [`Image::decorative`] quando ela não passa informação: sem nenhum dos dois, o lint acusa
/// `missing-alt`.
pub struct Image {
    common: Common,
    source: ImageSource,
    stretch: Stretch,
    corner_radius: f64,
    alt: Option<String>,
    decorative: bool,
    natural_size: Option<(u32, u32)>,
}

impl Image {
    /// Imagem dos bytes `bytes`: um `include_bytes!("logo.png")`, um `Vec<u8>` ou um
    /// `Arc<[u8]>`.
    pub fn bytes(bytes: impl Into<ImageSource>) -> Self {
        Self::source(bytes.into())
    }

    /// Imagem do arquivo `path`, lido agora (relativo à pasta em que o app roda).
    pub fn file(path: impl AsRef<Path>) -> Self {
        Self::source(ImageSource::file(path))
    }

    fn source(source: ImageSource) -> Self {
        Self {
            common: Common::default(),
            source,
            stretch: Stretch::default(),
            corner_radius: 0.0,
            alt: None,
            decorative: false,
            natural_size: None,
        }
    }

    /// O tamanho natural da imagem, em pixels, quando o app já o sabe (pelo `probe` do
    /// `tomni_ui::image`, numa thread, por exemplo). Sem `.width`/`.height`, a
    /// imagem mede então `min(width, largura disponível)` de largura e a altura na proporção,
    /// **sem decodificar** para medir: é a imagem "ajustada à largura da coluna". Sem este
    /// método, o tamanho natural é o que a decodificação dá.
    pub fn natural_size(mut self, width: u32, height: u32) -> Self {
        self.natural_size = Some((width, height));
        self
    }

    /// Como a imagem ocupa o retângulo.
    pub fn stretch(mut self, stretch: Stretch) -> Self {
        self.stretch = stretch;
        self
    }

    /// Raio dos cantos, em pixels.
    pub fn corner_radius(mut self, radius: impl Into<f64>) -> Self {
        self.corner_radius = radius.into().max(0.0);
        self
    }

    /// Texto alternativo: o que a imagem mostra, para quem não a vê (o `AutomationProperties.Name`
    /// do WinUI).
    pub fn alt(mut self, alt: impl Into<String>) -> Self {
        self.alt = Some(alt.into());
        self
    }

    /// Só enfeite, sem informação: dispensa o [`Image::alt`].
    pub fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }

    /// Texto alternativo, se tiver.
    pub fn alt_text(&self) -> Option<&str> {
        self.alt.as_deref()
    }

    /// Se é só enfeite.
    pub fn is_decorative(&self) -> bool {
        self.decorative
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> ImageVisual {
        ImageVisual {
            source: self.source.clone(),
            stretch: self.stretch,
            corner_radius: self.corner_radius,
            natural_size: self.natural_size,
        }
    }
}

common_builders!(Image);

impl Widget for Image {
    fn kind(&self) -> &'static str {
        "image"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Image(self.visual())
    }

    fn text(&self) -> Option<String> {
        self.alt.clone()
    }

    fn decorative(&self) -> bool {
        self.decorative
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("common", &self.common)
            .field("source", &self.source)
            .field("stretch", &self.stretch)
            .field("corner_radius", &self.corner_radius)
            .field("alt", &self.alt)
            .field("decorative", &self.decorative)
            .field("natural_size", &self.natural_size)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static PONTO: &[u8] = b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>";

    #[test]
    fn builders_e_look() {
        let imagem = Image::bytes(PONTO)
            .id("ponto")
            .stretch(Stretch::Fill)
            .corner_radius(4)
            .alt("Um ponto");
        assert_eq!(imagem.common().id.as_deref(), Some("ponto"));
        assert_eq!(imagem.alt_text(), Some("Um ponto"));
        assert_eq!(imagem.text().as_deref(), Some("Um ponto"));
        assert!(!imagem.is_decorative());
        let Look::Image(visual) = imagem.look() else {
            panic!("não é imagem");
        };
        assert_eq!(visual.stretch, Stretch::Fill);
        assert_eq!(visual.corner_radius, 4.0);
        assert_eq!(visual.source.bytes(), Ok(PONTO));
        assert_eq!(Stretch::default(), Stretch::Uniform);
        assert!(Image::bytes(PONTO).decorative().is_decorative());
    }

    #[test]
    fn mesmos_bytes_mesma_chave() {
        assert_eq!(ImageSource::from(PONTO), ImageSource::from(PONTO));
        assert_eq!(
            ImageSource::from(PONTO).key(),
            ImageSource::from(PONTO).key()
        );
        let a = ImageSource::from(PONTO.to_vec());
        let b = ImageSource::from(PONTO.to_vec());
        assert_ne!(a, b);
        assert_eq!(a.clone(), a);
    }

    #[test]
    fn arquivo_que_nao_existe_guarda_o_erro() {
        let imagem = Image::file("/nao/existe.png");
        let Look::Image(visual) = imagem.look() else {
            panic!("não é imagem");
        };
        let erro = visual.source.bytes().unwrap_err();
        assert!(
            erro.starts_with("não deu para ler /nao/existe.png"),
            "{erro}"
        );
    }
}
