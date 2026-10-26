//! Área de transferência do sistema.

#[cfg(not(target_os = "android"))]
use std::path::PathBuf;
use tomni_ui_core::{Clipboard, ClipboardImage, MemoryClipboard};

/// Clipboard do sistema no desktop, via `arboard`. Sem acesso a ele (no Android, ou no Linux
/// sem servidor gráfico), fica a [`MemoryClipboard`], só deste app.
pub fn system() -> Box<dyn Clipboard> {
    #[cfg(not(target_os = "android"))]
    if let Ok(clipboard) = arboard::Clipboard::new() {
        return Box::new(Arboard(clipboard));
    }
    Box::new(MemoryClipboard::default())
}

#[cfg(not(target_os = "android"))]
struct Arboard(arboard::Clipboard);

#[cfg(not(target_os = "android"))]
impl Clipboard for Arboard {
    fn read(&mut self) -> Option<String> {
        self.0.get_text().ok()
    }

    fn write(&mut self, text: String) {
        // Sem como avisar o usuário: o texto só não é copiado, como num `Ctrl+C` que falha.
        let _ = self.0.set_text(text);
    }

    fn read_files(&mut self) -> Option<Vec<PathBuf>> {
        self.0.get().file_list().ok().filter(|f| !f.is_empty())
    }

    fn read_image(&mut self) -> Option<ClipboardImage> {
        let image = self.0.get().image().ok()?;
        if image.width == 0 || image.height == 0 {
            return None;
        }
        Some(ClipboardImage {
            width: u32::try_from(image.width).ok()?,
            height: u32::try_from(image.height).ok()?,
            rgba: image.bytes.into_owned(),
        })
    }

    fn write_files(&mut self, files: Vec<PathBuf>) {
        let _ = self.0.set().file_list(&files);
    }

    fn write_image(&mut self, image: ClipboardImage) {
        let _ = self.0.set().image(arboard::ImageData {
            width: image.width as usize,
            height: image.height as usize,
            bytes: image.rgba.into(),
        });
    }
}
