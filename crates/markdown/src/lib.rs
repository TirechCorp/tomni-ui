//! Markdown para o Tomni UI: o [`MarkdownView`] que mostra o texto, o [`highlight`] que o
//! colore no `TextEditor`, as operações de edição de [`edit`] e os [`links`] do texto.
//!
//! ```
//! use tomni_ui_markdown::{highlight, MarkdownView};
//!
//! let fonte = "# Notas\n\n- [ ] ligar\n- [x] enviar\n";
//! let faixas = highlight(fonte);
//! assert!(!faixas.is_empty());
//! let _vista = MarkdownView::new(fonte).id("notas");
//! ```

pub mod edit;
mod highlight;
mod links;
mod parse;
mod view;

pub use highlight::highlight;
pub use links::{LinkRef, links};
pub use view::{FenceCx, ImageCx, MarkdownView};
