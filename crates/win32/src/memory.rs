//! A memória da máquina, para a lista de processos do app.

use std::io;

use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

use crate::io_error;

/// A memória física da máquina ([`memory_status`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryStatus {
    /// A memória física instalada que o sistema enxerga, em bytes.
    pub total: u64,
    /// A que os programas podem usar sem o sistema tirar a de outros (a livre e a em espera,
    /// como o "Disponível" do Gerenciador de Tarefas), em bytes.
    pub available: u64,
}

/// `GlobalMemoryStatusEx`: o total e o disponível da memória física.
pub fn memory_status() -> io::Result<MemoryStatus> {
    let mut estado = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `estado` é local, vive durante a chamada e tem o `dwLength` igual ao tamanho dela.
    unsafe { GlobalMemoryStatusEx(&mut estado) }.map_err(io_error)?;
    Ok(MemoryStatus {
        total: estado.ullTotalPhys,
        available: estado.ullAvailPhys,
    })
}
