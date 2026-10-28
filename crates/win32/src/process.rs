//! A tabela de processos, para montar a cadeia de quem pediu a chave, e o uso de CPU e de
//! memória de um processo, para o medidor do app.

use std::io;
use std::time::Duration;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
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

/// O que um processo usou até agora ([`process_usage`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessUsage {
    /// O tempo de CPU, somando o modo usuário e o do sistema, de todas as threads.
    pub cpu: Duration,
    /// A memória em uso (o working set), em bytes.
    pub memory: u64,
}

/// `GetProcessTimes` e `GetProcessMemoryInfo` do processo `pid` (o próprio app, com
/// `std::process::id()`, ou um filho, como o shell de uma aba). Falha se o processo acabou ou
/// se não há permissão para olhar.
pub fn process_usage(pid: u32) -> io::Result<ProcessUsage> {
    // SAFETY: a chamada só abre um handle novo (ou devolve um erro); o `Handle` o fecha.
    let processo =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.map_err(io_error)?;
    let processo = Handle(processo);
    let mut criado = FILETIME::default();
    let mut saiu = FILETIME::default();
    let mut sistema = FILETIME::default();
    let mut usuario = FILETIME::default();
    // SAFETY: `processo` é um handle válido com acesso de consulta, e os quatro ponteiros são
    // de variáveis locais vivas durante a chamada.
    unsafe {
        GetProcessTimes(
            processo.0,
            &mut criado,
            &mut saiu,
            &mut sistema,
            &mut usuario,
        )
    }
    .map_err(io_error)?;
    let mut contadores = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: o mesmo handle válido; `contadores` é local, com o `cb` igual ao tamanho passado.
    unsafe { GetProcessMemoryInfo(processo.0, &mut contadores, contadores.cb) }
        .map_err(io_error)?;
    Ok(ProcessUsage {
        cpu: intervalo(sistema) + intervalo(usuario),
        memory: contadores.WorkingSetSize as u64,
    })
}

/// Um `FILETIME` de duração (em unidades de 100 ns) como `Duration`.
fn intervalo(tempo: FILETIME) -> Duration {
    let unidades = (u64::from(tempo.dwHighDateTime) << 32) | u64::from(tempo.dwLowDateTime);
    Duration::from_nanos(unidades.saturating_mul(100))
}
