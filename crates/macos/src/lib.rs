//! As chamadas do macOS que nenhum crate expõe com API segura. Com o `tomni-ui-win32`, é um dos
//! dois crates com `unsafe`.
//!
//! O medidor de CPU do app lê o uso de CPU e de memória do próprio processo e dos programas dos
//! shells locais pelo `proc_pidinfo` (`PROC_PIDTASKINFO`), acha os filhos de um processo pelo
//! `proc_listchildpids` e converte o tempo com o `mach_timebase_info`. A lista de processos do
//! app soma o `proc_listallpids`, o `PROC_PIDTBSDINFO`, o `proc_pid_rusage` (o disco), o
//! `proc_pidpath` e o `getpwuid_r` (o nome do dono). Todas são da `libc` e só existem como
//! `unsafe extern "C"`; o usuário aprovou o `unsafe` contido aqui em 29/10.
//!
//! Cada bloco `unsafe` tem um `// SAFETY:` que diz por que a chamada é válida, tem uma única
//! operação insegura, e nenhuma função pública do crate é `unsafe`.
#![cfg(target_os = "macos")]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

mod process;

#[cfg(test)]
mod tests;

pub use process::{
    DiskUsage, ProcessInfo, ProcessState, ProcessUsage, all_pids, child_pids, process_info,
    process_path, process_usage, user_name,
};
