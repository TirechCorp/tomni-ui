//! O tempo de cada núcleo, para o gráfico por núcleo e a linha do tempo de kernel do app.

use std::io;
use std::time::Duration;

use windows::Wdk::System::SystemInformation::{
    NtQuerySystemInformation, SystemProcessorPerformanceInformation,
};
use windows::Win32::System::WindowsProgramming::SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION;

use crate::io_error;

/// Quantos núcleos lógicos cabem na leitura: o `NtQuerySystemInformation` só dá os do grupo de
/// processadores do processo, que tem no máximo 64.
const MAXIMO: usize = 256;

/// O tempo que um núcleo lógico passou em cada modo desde o boot ([`cpu_times`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    /// No modo usuário.
    pub user: Duration,
    /// No kernel, sem o tempo ocioso (que o Windows conta como kernel).
    pub kernel: Duration,
    /// Ocioso.
    pub idle: Duration,
}

/// `NtQuerySystemInformation` com `SystemProcessorPerformanceInformation`: o tempo de cada
/// núcleo lógico do grupo de processadores do processo, na ordem do sistema.
pub fn cpu_times() -> io::Result<Vec<CpuTimes>> {
    let mut lista = vec![SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION::default(); MAXIMO];
    let mut escritos = 0u32;
    let tamanho = size_of_val(lista.as_slice()) as u32;
    // SAFETY: `lista` é local, vive durante a chamada e tem os `tamanho` bytes que ela pode
    // escrever; `escritos` também é local.
    let status = unsafe {
        NtQuerySystemInformation(
            SystemProcessorPerformanceInformation,
            lista.as_mut_ptr().cast(),
            tamanho,
            &mut escritos,
        )
    };
    status.ok().map_err(io_error)?;
    lista.truncate(escritos as usize / size_of::<SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION>());
    Ok(lista.iter().map(tempos).collect())
}

/// Os tempos de uma entrada, que vêm em unidades de 100 ns.
pub(crate) fn tempos(info: &SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION) -> CpuTimes {
    let duracao = |cem_ns: i64| Duration::from_nanos(u64::try_from(cem_ns).unwrap_or(0) * 100);
    CpuTimes {
        user: duracao(info.UserTime),
        kernel: duracao(info.KernelTime.saturating_sub(info.IdleTime)),
        idle: duracao(info.IdleTime),
    }
}
