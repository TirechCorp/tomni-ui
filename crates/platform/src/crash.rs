//! O registro das quedas: o pânico e o erro que fecham o app vão para um arquivo e para uma
//! caixa de aviso do sistema.
//!
//! No Windows, o app do subsistema gráfico não tem console: o `eprintln!` e a mensagem padrão
//! do pânico somem, e quem usa só vê a janela fechar. Com [`CrashReport::install`], o pânico
//! é acrescentado ao [`log`](CrashReport::log), com o rastro da pilha, e a thread principal
//! mostra o aviso.

use std::backtrace::Backtrace;
use std::fmt::Display;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// O arquivo passa disto e recomeça: guarda as últimas quedas, não o histórico todo.
const LOG_LIMIT: u64 = 512 * 1024;

/// Já mostrou o aviso: um pânico dentro do aviso, ou outro logo depois, só grava.
static WARNED: AtomicBool = AtomicBool::new(false);

/// Onde gravar e o que dizer quando o app cai.
#[derive(Clone, Debug)]
pub struct CrashReport {
    /// O arquivo que recebe cada queda, acrescentada ao fim.
    pub log: PathBuf,
    /// O título da caixa de aviso.
    pub title: String,
    /// O texto da caixa de aviso: o que aconteceu, onde está o [`log`](Self::log) e o que fazer.
    pub message: String,
}

impl CrashReport {
    /// Grava os pânicos de todas as threads no [`log`](Self::log); o da thread principal, que
    /// fecha o app, também mostra o aviso. A mensagem padrão do Rust continua saindo no
    /// stderr.
    pub fn install(self) {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            previous(info);
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("sem nome");
            self.record(&format!(
                "pânico na thread {name}: {info}\n{}",
                Backtrace::force_capture()
            ));
            if name == "main" {
                self.warn();
            }
        }));
    }

    /// Grava `error`, que fez o app terminar sem pânico (a janela que não abriu), e mostra o
    /// aviso.
    pub fn failure(&self, error: &dyn Display) {
        self.record(&format!("erro: {error}"));
        self.warn();
    }

    fn record(&self, text: &str) {
        if let Some(dir) = self.log.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let full = fs::metadata(&self.log).is_ok_and(|m| m.len() > LOG_LIMIT);
        let file = OpenOptions::new()
            .create(true)
            .append(!full)
            .write(true)
            .truncate(full)
            .open(&self.log);
        let version = env!("CARGO_PKG_VERSION");
        let written = file.and_then(|mut file| {
            writeln!(
                file,
                "--- {} (tomni-ui {version}, {} {})\n{text}\n",
                jiff::Timestamp::now(),
                std::env::consts::OS,
                std::env::consts::ARCH,
            )
        });
        if let Err(e) = written {
            eprintln!(
                "tomni-ui: a queda não foi gravada em {}: {e}",
                self.log.display()
            );
        }
    }

    fn warn(&self) {
        if WARNED.swap(true, Ordering::SeqCst) {
            return;
        }
        alert(&self.title, &self.message);
    }
}

/// Mostra uma caixa de aviso do sistema, com o botão OK, e espera fechar. Serve fora da
/// janela: antes de ela abrir (o app que não pode iniciar) ou depois de ela cair.
pub fn alert(title: &str, message: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relatorio(nome: &str) -> CrashReport {
        let dir =
            std::env::temp_dir().join(format!("tomni-ui-queda-{}-{nome}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        CrashReport {
            log: dir.join("pasta").join("quedas.log"),
            title: String::new(),
            message: String::new(),
        }
    }

    #[test]
    fn cada_queda_vai_para_o_fim_do_arquivo() {
        let report = relatorio("fim");
        report.record("primeira");
        report.record("segunda");
        let texto = fs::read_to_string(&report.log).unwrap();
        let (a, b) = (
            texto.find("primeira").unwrap(),
            texto.find("segunda").unwrap(),
        );
        assert!(a < b, "{texto}");
        assert_eq!(texto.matches("--- ").count(), 2, "{texto}");
        assert!(texto.contains(std::env::consts::OS), "{texto}");
    }

    #[test]
    fn arquivo_grande_demais_recomeca() {
        let report = relatorio("grande");
        fs::create_dir_all(report.log.parent().unwrap()).unwrap();
        fs::write(&report.log, vec![b'x'; LOG_LIMIT as usize + 1]).unwrap();
        report.record("nova");
        let texto = fs::read_to_string(&report.log).unwrap();
        assert!(!texto.contains("xxxx"), "o arquivo antigo foi descartado");
        assert!(texto.contains("nova"));
    }
}
