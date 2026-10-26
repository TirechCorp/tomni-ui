//! Processos filhos sem janela de console (M65).
//!
//! No Windows, um app de interface que abre um programa de console (`rclone`, `xauth`,
//! `powershell`…) faz o sistema criar uma janela de console para ele, que pisca na tela. A
//! lib e os apps passam o `Command` por [`no_window`] antes de rodá-lo.

use std::process::Command;

/// `CREATE_NO_WINDOW` do Windows: o processo de console roda sem criar janela.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Faz o processo de console abrir sem janela. No Windows, põe `CREATE_NO_WINDOW` nas flags de
/// criação; nos outros sistemas não faz nada. Devolve o próprio `Command`, para encadear:
///
/// ```
/// let mut comando = std::process::Command::new("hostname");
/// tomni_ui_core::process::no_window(&mut comando);
/// ```
pub fn no_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devolve_o_mesmo_comando_para_encadear() {
        let mut comando = Command::new("programa-que-nao-existe");
        let ponteiro: *const Command = &comando;
        let devolvido = no_window(&mut comando).arg("-x") as *const Command;
        assert_eq!(ponteiro, devolvido);
    }

    #[cfg(windows)]
    #[test]
    fn no_windows_um_processo_de_console_roda_sem_janela() {
        // Só compila no Windows: o importante é a conferência na VM.
        let saida = no_window(&mut Command::new("cmd.exe").args(["/C", "echo ok"]))
            .output()
            .expect("cmd.exe");
        assert!(saida.status.success());
    }
}
