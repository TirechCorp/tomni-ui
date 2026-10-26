//! A tabela de processos, para montar a cadeia de quem pediu a chave.

use std::io;

use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

use crate::{Handle, io_error};

/// Um processo da tabela.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessEntry {
    /// O pid do processo.
    pub pid: u32,
    /// O pid do pai (que pode já ter terminado).
    pub parent_pid: u32,
    /// O nome do executável, sem a pasta (`ssh.exe`).
    pub name: String,
}

/// `CreateToolhelp32Snapshot`: o pid, o pid do pai e o nome do executável de cada processo.
pub fn process_table() -> io::Result<Vec<ProcessEntry>> {
    // SAFETY: a chamada só lê o estado do sistema e devolve um handle novo (ou um erro).
    let foto = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.map_err(io_error)?;
    let foto = Handle(foto);
    let mut entrada = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut processos = Vec::new();
    // SAFETY: `foto` é um handle de snapshot válido e `entrada` tem o `dwSize` preenchido.
    let mut tem = unsafe { Process32FirstW(foto.0, &mut entrada) }.is_ok();
    while tem {
        let fim = entrada
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entrada.szExeFile.len());
        processos.push(ProcessEntry {
            pid: entrada.th32ProcessID,
            parent_pid: entrada.th32ParentProcessID,
            name: String::from_utf16_lossy(&entrada.szExeFile[..fim]),
        });
        // SAFETY: o mesmo handle e a mesma `entrada` válidos da chamada anterior.
        tem = unsafe { Process32NextW(foto.0, &mut entrada) }.is_ok();
    }
    Ok(processos)
}
