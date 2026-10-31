//! A tabela de processos, para montar a cadeia de quem pediu a chave e para a lista de
//! processos do app; o uso de CPU, de memória e de disco de um processo, o caminho do
//! executável e o usuário dono.

use std::io;
use std::time::Duration;

use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::Security::{
    GetTokenInformation, LookupAccountSidW, SID_NAME_USE, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GetProcessIoCounters, GetProcessTimes, IO_COUNTERS, OpenProcess, OpenProcessToken,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{PCWSTR, PWSTR};

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
    /// Quantas threads o processo tem.
    pub threads: u32,
}

/// `CreateToolhelp32Snapshot`: o pid, o pid do pai, o nome do executável e as threads de cada
/// processo.
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
            threads: entrada.cntThreads,
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
    /// Os bytes lidos até agora (`GetProcessIoCounters`: arquivos, rede e dispositivos, sem
    /// separar); zero se o sistema não disse.
    pub read_bytes: u64,
    /// Os bytes escritos até agora, como em `read_bytes`.
    pub written_bytes: u64,
    /// As operações de leitura até agora, contadas como os `read_bytes`.
    pub read_operations: u64,
    /// As operações de escrita até agora.
    pub write_operations: u64,
    /// Quando o processo começou, em unidades de 100 ns desde 1601 (o `FILETIME`). Com o pid,
    /// diz se é o mesmo processo de uma leitura anterior (o Windows reaproveita os pids).
    pub start_time: u64,
}

/// Abre o processo `pid` só para consultas.
fn abrir(pid: u32) -> io::Result<Handle> {
    // SAFETY: a chamada só abre um handle novo (ou devolve um erro); o `Handle` o fecha.
    let processo =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.map_err(io_error)?;
    Ok(Handle(processo))
}

/// `GetProcessTimes`, `GetProcessMemoryInfo` e `GetProcessIoCounters` do processo `pid` (o
/// próprio app, com `std::process::id()`, um filho, como o shell de uma aba, ou qualquer um da
/// [`process_table`]). Falha se o processo acabou ou se não há permissão para olhar (os
/// protegidos do sistema).
pub fn process_usage(pid: u32) -> io::Result<ProcessUsage> {
    let processo = abrir(pid)?;
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
    let mut es = IO_COUNTERS::default();
    // SAFETY: o mesmo handle válido; `es` é local e vive durante a chamada. Se falhar, os
    // contadores ficam em zero.
    let _ = unsafe { GetProcessIoCounters(processo.0, &mut es) };
    Ok(ProcessUsage {
        cpu: intervalo(sistema) + intervalo(usuario),
        memory: contadores.WorkingSetSize as u64,
        read_bytes: es.ReadTransferCount,
        written_bytes: es.WriteTransferCount,
        read_operations: es.ReadOperationCount,
        write_operations: es.WriteOperationCount,
        start_time: (u64::from(criado.dwHighDateTime) << 32) | u64::from(criado.dwLowDateTime),
    })
}

/// `QueryFullProcessImageNameW`: o caminho completo do executável do processo `pid`
/// (`C:\Windows\System32\svchost.exe`). Falha como o [`process_usage`].
pub fn process_path(pid: u32) -> io::Result<String> {
    let processo = abrir(pid)?;
    // O maior caminho que o Windows aceita.
    let mut caminho = vec![0u16; 32_768];
    let mut tamanho = caminho.len() as u32;
    // SAFETY: `caminho` tem `tamanho` unidades e vive durante a chamada, que escreve no máximo
    // isso e devolve em `tamanho` quantas escreveu (sem o NUL).
    unsafe {
        QueryFullProcessImageNameW(
            processo.0,
            PROCESS_NAME_WIN32,
            PWSTR(caminho.as_mut_ptr()),
            &mut tamanho,
        )
    }
    .map_err(io_error)?;
    caminho.truncate(tamanho as usize);
    Ok(String::from_utf16_lossy(&caminho))
}

/// O usuário dono do processo `pid`, sem o domínio (`ana`, `SYSTEM`): o `TokenUser` do token do
/// processo, pelo `LookupAccountSidW`. Falha se o processo acabou ou se não há permissão para
/// abrir o token (os de outros usuários, sem ser administrador).
pub fn process_user(pid: u32) -> io::Result<String> {
    let processo = abrir(pid)?;
    let mut token = HANDLE::default();
    // SAFETY: `processo` é um handle válido; `token` é local e recebe um handle novo, que o
    // `Handle` fecha.
    unsafe { OpenProcessToken(processo.0, TOKEN_QUERY, &mut token) }.map_err(io_error)?;
    let token = Handle(token);
    let mut tamanho = 0u32;
    // SAFETY: sem buffer, a chamada só devolve o tamanho necessário em `tamanho` (falha com
    // `ERROR_INSUFFICIENT_BUFFER`, que é o esperado).
    let _ = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut tamanho) };
    // Em `u64`, para o `TOKEN_USER` (que começa com um ponteiro) ficar alinhado.
    let mut dados = vec![0u64; (tamanho as usize).div_ceil(8).max(1)];
    let capacidade = (dados.len() * 8) as u32;
    // SAFETY: `dados` tem `capacidade` bytes e vive durante a chamada, que escreve no máximo
    // isso.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(dados.as_mut_ptr().cast()),
            capacidade,
            &mut tamanho,
        )
    }
    .map_err(io_error)?;
    // SAFETY: a chamada deu certo e escreveu um `TOKEN_USER` no começo de `dados`, que está
    // alinhado para ele (`u64`) e continua vivo (o SID aponta para dentro dele).
    let sid = unsafe { dados.as_ptr().cast::<TOKEN_USER>().read() }
        .User
        .Sid;
    let mut nome = [0u16; 256];
    let mut tamanho_nome = nome.len() as u32;
    let mut dominio = [0u16; 256];
    let mut tamanho_dominio = dominio.len() as u32;
    let mut uso = SID_NAME_USE::default();
    // SAFETY: `sid` aponta para dentro de `dados`, ainda vivo; os dois buffers têm os tamanhos
    // passados, e `uso` é local.
    unsafe {
        LookupAccountSidW(
            PCWSTR::null(),
            sid,
            Some(PWSTR(nome.as_mut_ptr())),
            &mut tamanho_nome,
            Some(PWSTR(dominio.as_mut_ptr())),
            &mut tamanho_dominio,
            &mut uso,
        )
    }
    .map_err(io_error)?;
    Ok(String::from_utf16_lossy(&nome[..tamanho_nome as usize]))
}

/// Um `FILETIME` de duração (em unidades de 100 ns) como `Duration`.
fn intervalo(tempo: FILETIME) -> Duration {
    let unidades = (u64::from(tempo.dwHighDateTime) << 32) | u64::from(tempo.dwLowDateTime);
    Duration::from_nanos(unidades.saturating_mul(100))
}
