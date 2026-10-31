//! As chamadas do macOS que nenhum crate expõe com API segura. Com o `tomni-ui-win32`, é um dos
//! dois crates com `unsafe`.
//!
//! O medidor de CPU do app lê o uso de CPU e de memória do próprio processo e dos programas dos
//! shells locais pelo `proc_pidinfo` (`PROC_PIDTASKINFO`), acha os filhos de um processo pelo
//! `proc_listchildpids` e converte o tempo com o `mach_timebase_info`. A lista de processos do
//! app soma o `proc_listallpids`, o `PROC_PIDTBSDINFO`, o `proc_pid_rusage` (o disco), o
//! `proc_pidpath` e o `getpwuid_r` (o nome do dono), e a memória da máquina pelo `sysctlbyname`
//! (`hw.memsize`), o `host_statistics64` (`HOST_VM_INFO64`, com o `mach_host_self`) e o
//! `sysconf`. O gráfico por núcleo lê o `host_processor_info` (`PROCESSOR_CPU_LOAD_INFO`, solto
//! pelo `vm_deallocate`) e a aba Rede, o `sysctl` com `NET_RT_IFLIST2` e o `if_indextoname`.
//! Todas são da `libc` e só existem como `unsafe extern "C"`; o usuário aprovou o `unsafe`
//! contido aqui em 29/10. A aba Disco lê as `Statistics` de cada `IOBlockStorageDriver` pelo
//! IOKit (`IOServiceGetMatchingServices`, `IORegistryEntryGetParentEntry` e
//! `IORegistryEntryCreateCFProperties`, declaradas aqui), com os dicionários pelo
//! `core-foundation`.
//!
//! Cada bloco `unsafe` tem um `// SAFETY:` que diz por que a chamada é válida, tem uma única
//! operação insegura, e nenhuma função pública do crate é `unsafe`.
#![cfg(target_os = "macos")]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

mod cpu;
mod disk;
mod memory;
mod network;
mod process;

#[cfg(test)]
mod tests;

pub use cpu::{CpuTimes, cpu_times};
pub use disk::{DiskCounters, disk_counters};
pub use memory::{MemoryStatus, memory_status};
pub use network::{NetworkInterface, network_interfaces};
pub use process::{
    DiskUsage, ProcessInfo, ProcessState, ProcessUsage, all_pids, child_pids, process_info,
    process_path, process_usage, user_name,
};
