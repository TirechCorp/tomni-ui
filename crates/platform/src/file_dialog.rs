//! O seletor de arquivo de verdade, com o `rfd`. No Linux usa o portal XDG (ou o `zenity`); no
//! Windows e no macOS, o diálogo nativo.
//!
//! O diálogo roda numa thread própria, para não travar o loop de eventos, e a resposta volta
//! por [`FileDialogAnswer::send`], que acorda o loop.

use std::sync::Arc;

use tomni_ui_core::dialogs::{
    FileDialogAnswer, FileDialogKind, FileDialogRequest, FileDialogService,
};
use winit::window::Window;

/// Quem mostra os diálogos do app, preso à janela dele.
pub(crate) struct SystemFileDialogs {
    window: Arc<Window>,
}

impl SystemFileDialogs {
    pub(crate) fn new(window: Arc<Window>) -> Self {
        Self { window }
    }
}

impl FileDialogService for SystemFileDialogs {
    fn show(&self, request: FileDialogRequest, answer: FileDialogAnswer) {
        let mut dialog = rfd::AsyncFileDialog::new().set_parent(&*self.window);
        if !request.title.is_empty() {
            dialog = dialog.set_title(&request.title);
        }
        for filter in &request.filters {
            dialog = dialog.add_filter(&filter.name, &filter.extensions);
        }
        if let Some(directory) = &request.directory {
            dialog = dialog.set_directory(directory);
        }
        if let Some(name) = &request.file_name {
            dialog = dialog.set_file_name(name);
        }
        std::thread::spawn(move || {
            let paths = pollster::block_on(async {
                let um = |handle: Option<rfd::FileHandle>| -> Vec<_> {
                    handle.map(|h| h.path().to_path_buf()).into_iter().collect()
                };
                match request.kind {
                    FileDialogKind::Open => um(dialog.pick_file().await),
                    FileDialogKind::OpenMany => dialog
                        .pick_files()
                        .await
                        .unwrap_or_default()
                        .iter()
                        .map(|h| h.path().to_path_buf())
                        .collect(),
                    FileDialogKind::Save => um(dialog.save_file().await),
                    FileDialogKind::Folder => um(dialog.pick_folder().await),
                }
            });
            answer.send_many(paths);
        });
    }
}
