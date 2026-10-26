//! O que os testes do `tomni-ui-files` dividem.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tomni_ui_files::Reply;

static CONTADOR: AtomicUsize = AtomicUsize::new(0);

/// Uma pasta temporária que some quando o teste acaba.
pub struct Pasta(PathBuf);

impl Pasta {
    pub fn nova(nome: &str) -> Pasta {
        let n = CONTADOR.fetch_add(1, Ordering::SeqCst);
        let caminho =
            std::env::temp_dir().join(format!("fluent-files-{nome}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&caminho);
        std::fs::create_dir_all(&caminho).unwrap();
        Pasta(caminho)
    }

    pub fn caminho(&self) -> &Path {
        &self.0
    }

    /// O caminho de um item, no formato do trait (com `/`).
    pub fn texto(&self, nome: &str) -> String {
        self.0.join(nome).to_string_lossy().replace('\\', "/")
    }

    pub fn escrever(&self, nome: &str, dados: &[u8]) -> PathBuf {
        let caminho = self.0.join(nome);
        std::fs::create_dir_all(caminho.parent().unwrap()).unwrap();
        std::fs::write(&caminho, dados).unwrap();
        caminho
    }
}

impl Drop for Pasta {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Espera a resposta de um pedido da `Remote` (até 10 s).
pub fn esperar<T: Send + Clone + 'static>(resposta: &Reply<T>) -> Result<T, String> {
    let limite = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(valor) = resposta.get_untracked() {
            return valor;
        }
        assert!(Instant::now() < limite, "a resposta não chegou");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Dados que não se repetem em blocos curtos (para ver trocas de ordem).
pub fn dados(tamanho: usize) -> Vec<u8> {
    (0..tamanho)
        .map(|i| ((i * 31 + i / 251) % 256) as u8)
        .collect()
}

#[cfg(all(unix, feature = "sftp"))]
pub mod servidor;

#[cfg(all(unix, feature = "ftp"))]
pub mod ftpd;

#[cfg(all(unix, feature = "s3"))]
pub mod s3d;

#[cfg(feature = "rclone")]
pub mod rcd_falso;
