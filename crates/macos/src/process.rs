//! O uso de CPU e de memória de um processo e os filhos dele, para o medidor do app, e a lista
//! de todos os processos da máquina, com o disco de cada um, para a lista de processos.

use std::ffi::CStr;
use std::io;
use std::sync::OnceLock;
use std::time::Duration;

/// O que um processo usou até agora.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessUsage {
    /// O tempo de CPU, somando o modo usuário e o do sistema, de todas as threads (inclusive as
    /// que já terminaram).
    pub cpu: Duration,
    /// A memória em uso (a residente), em bytes.
    pub memory: u64,
}

/// `proc_pidinfo` com `PROC_PIDTASKINFO` do processo `pid` (o próprio app, com
/// `std::process::id()`, ou um filho, como o shell de uma aba). Falha se o processo acabou ou
/// se não há permissão para olhar (o de outro usuário, como um `sudo`).
pub fn process_usage(pid: u32) -> io::Result<ProcessUsage> {
    let pid = i32::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::zeroed();
    let tamanho = size_of::<libc::proc_taskinfo>() as i32;
    // SAFETY: `info` é local, vive durante a chamada e tem os `tamanho` bytes que a chamada
    // pode escrever; o `pid` errado só faz a chamada falhar.
    let escrito = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            tamanho,
        )
    };
    if escrito <= 0 {
        return Err(io::Error::last_os_error());
    }
    if escrito != tamanho {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
    }
    // SAFETY: a chamada escreveu a estrutura inteira (`escrito == tamanho`), e a de zeros já era
    // válida (só inteiros).
    let info = unsafe { info.assume_init() };
    Ok(ProcessUsage {
        cpu: de_tiques(info.pti_total_user.saturating_add(info.pti_total_system)),
        memory: info.pti_resident_size,
    })
}

/// `proc_listchildpids`: os pids dos filhos diretos de `pid`. Vazio se não há filhos ou se o
/// processo acabou.
pub fn child_pids(pid: u32) -> io::Result<Vec<u32>> {
    let pid = i32::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut capacidade = 64;
    loop {
        let mut pids = vec![0i32; capacidade];
        let bytes = i32::try_from(capacidade * size_of::<i32>())
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        // SAFETY: `pids` tem `bytes` bytes e vive durante a chamada, que escreve no máximo isso.
        let n = unsafe { libc::proc_listchildpids(pid, pids.as_mut_ptr().cast(), bytes) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        // A conta volta em pids; se encheu o espaço, pode haver mais: tenta com o dobro.
        let n = n as usize;
        if n >= capacidade && capacidade < 1 << 16 {
            capacidade *= 2;
            continue;
        }
        pids.truncate(n.min(capacidade));
        return Ok(pids
            .into_iter()
            .filter_map(|p| u32::try_from(p).ok().filter(|&p| p > 0))
            .collect());
    }
}

/// O estado de um processo, do `pbi_status` do `proc_bsdinfo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    /// Nascendo (`SIDL`).
    Idle,
    /// Rodando ou pronto para rodar (`SRUN`).
    Running,
    /// Esperando um evento (`SSLEEP`).
    Sleeping,
    /// Parado por um sinal ou pelo depurador (`SSTOP`).
    Stopped,
    /// Acabou e espera o pai ler o fim (`SZOMB`).
    Zombie,
    /// Um valor que o sistema não documenta.
    Unknown,
}

/// Os bytes que um processo leu e escreveu no disco até agora.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskUsage {
    /// Os bytes lidos do disco.
    pub read_bytes: u64,
    /// Os bytes escritos no disco.
    pub written_bytes: u64,
}

/// Um processo da máquina ([`process_info`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessInfo {
    /// O pid.
    pub pid: u32,
    /// O pid do pai.
    pub parent_pid: u32,
    /// O usuário dono (o efetivo); o nome vem do [`user_name`].
    pub uid: u32,
    /// O nome do processo (até 32 letras, o `pbi_name`, ou o `pbi_comm`).
    pub name: String,
    /// O estado.
    pub state: ProcessState,
    /// Quando o processo começou, em microssegundos desde 1970. Com o pid, diz se é o mesmo
    /// processo de uma leitura anterior.
    pub start_time: u64,
    /// As threads, a CPU e a memória (`PROC_PIDTASKINFO`); `None` se não há permissão (o
    /// processo de outro usuário, sem ser root).
    pub usage: Option<ProcessUsage>,
    /// Quantas threads o processo tem, com a mesma permissão do `usage`.
    pub threads: Option<u32>,
    /// O disco (`proc_pid_rusage`), com a mesma permissão do `usage`.
    pub disk: Option<DiskUsage>,
}

/// `proc_listallpids`: os pids de todos os processos da máquina.
pub fn all_pids() -> io::Result<Vec<u32>> {
    // SAFETY: sem buffer, a chamada só devolve quantos processos há.
    let quantos = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if quantos < 0 {
        return Err(io::Error::last_os_error());
    }
    // Uma folga para os que nascerem entre as duas chamadas.
    let capacidade = quantos as usize + 64;
    let mut pids = vec![0i32; capacidade];
    let bytes = i32::try_from(capacidade * size_of::<i32>())
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    // SAFETY: `pids` tem `bytes` bytes e vive durante a chamada, que escreve no máximo isso.
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    pids.truncate((n as usize).min(capacidade));
    Ok(pids
        .into_iter()
        .filter_map(|p| u32::try_from(p).ok().filter(|&p| p > 0))
        .collect())
}

/// `proc_pidinfo` com `PROC_PIDTBSDINFO` e `PROC_PIDTASKINFO` e `proc_pid_rusage` do processo
/// `pid`. Falha se o processo acabou; sem permissão para o resto, só os dados do
/// `proc_bsdinfo` (o nome, o pai, o dono e o estado) vêm.
pub fn process_info(pid: u32) -> io::Result<ProcessInfo> {
    let pid_c = i32::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let tamanho = size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: `info` é local, vive durante a chamada e tem os `tamanho` bytes que a chamada
    // pode escrever; o `pid` errado só faz a chamada falhar.
    let escrito = unsafe {
        libc::proc_pidinfo(
            pid_c,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            tamanho,
        )
    };
    if escrito <= 0 {
        return Err(io::Error::last_os_error());
    }
    if escrito != tamanho {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
    }
    // SAFETY: a chamada escreveu a estrutura inteira, e a de zeros já era válida (só inteiros
    // e vetores de `c_char`).
    let info = unsafe { info.assume_init() };
    let nome = texto(&info.pbi_name);
    let tarefa = tarefa(pid_c).ok();
    Ok(ProcessInfo {
        pid,
        parent_pid: info.pbi_ppid,
        uid: info.pbi_uid,
        name: if nome.is_empty() {
            texto(&info.pbi_comm)
        } else {
            nome
        },
        state: match info.pbi_status {
            1 => ProcessState::Idle,
            2 => ProcessState::Running,
            3 => ProcessState::Sleeping,
            4 => ProcessState::Stopped,
            5 => ProcessState::Zombie,
            _ => ProcessState::Unknown,
        },
        start_time: info
            .pbi_start_tvsec
            .saturating_mul(1_000_000)
            .saturating_add(info.pbi_start_tvusec),
        usage: tarefa.map(|t| ProcessUsage {
            cpu: de_tiques(t.pti_total_user.saturating_add(t.pti_total_system)),
            memory: t.pti_resident_size,
        }),
        threads: tarefa.map(|t| u32::try_from(t.pti_threadnum).unwrap_or(0)),
        disk: disco(pid_c).ok(),
    })
}

/// O texto de um vetor de `c_char` terminado em NUL (ou cheio).
fn texto(letras: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = letras
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `PROC_PIDTASKINFO` do processo `pid`.
fn tarefa(pid: i32) -> io::Result<libc::proc_taskinfo> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::zeroed();
    let tamanho = size_of::<libc::proc_taskinfo>() as i32;
    // SAFETY: `info` é local, vive durante a chamada e tem os `tamanho` bytes que a chamada
    // pode escrever; o `pid` errado só faz a chamada falhar.
    let escrito = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            tamanho,
        )
    };
    if escrito <= 0 {
        return Err(io::Error::last_os_error());
    }
    if escrito != tamanho {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
    }
    // SAFETY: a chamada escreveu a estrutura inteira (`escrito == tamanho`), e a de zeros já era
    // válida (só inteiros).
    Ok(unsafe { info.assume_init() })
}

/// `proc_pid_rusage` com `RUSAGE_INFO_V2`: o disco do processo `pid`.
fn disco(pid: i32) -> io::Result<DiskUsage> {
    let mut uso = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: `uso` é local, vive durante a chamada e é do tamanho da versão pedida (a 2), que
    // é o que a chamada escreve.
    let erro = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, uso.as_mut_ptr().cast()) };
    if erro != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a chamada deu certo e preencheu a estrutura, e a de zeros já era válida (só
    // inteiros).
    let uso = unsafe { uso.assume_init() };
    Ok(DiskUsage {
        read_bytes: uso.ri_diskio_bytesread,
        written_bytes: uso.ri_diskio_byteswritten,
    })
}

/// `proc_pidpath`: o caminho completo do executável do processo `pid`. Falha se o processo
/// acabou ou se não há permissão.
pub fn process_path(pid: u32) -> io::Result<String> {
    let pid = i32::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // O `PROC_PIDPATHINFO_MAXSIZE` (4 × `MAXPATHLEN`).
    let mut caminho = vec![0u8; 4 * 1024];
    // SAFETY: `caminho` tem o tamanho passado e vive durante a chamada, que escreve no máximo
    // isso.
    let n = unsafe { libc::proc_pidpath(pid, caminho.as_mut_ptr().cast(), caminho.len() as u32) };
    if n <= 0 {
        return Err(io::Error::last_os_error());
    }
    caminho.truncate(n as usize);
    Ok(String::from_utf8_lossy(&caminho).into_owned())
}

/// `getpwuid_r`: o nome do usuário `uid` (`ana`, `root`, `_windowserver`); `None` se o sistema
/// não conhece.
pub fn user_name(uid: u32) -> Option<String> {
    let mut registro = std::mem::MaybeUninit::<libc::passwd>::zeroed();
    let mut espaco = vec![0 as libc::c_char; 16 * 1024];
    let mut achado: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: `registro`, `espaco` (com o tamanho passado) e `achado` são locais e vivem durante
    // a chamada, que escreve o registro e as cadeias dele dentro do `espaco`.
    let erro = unsafe {
        libc::getpwuid_r(
            uid,
            registro.as_mut_ptr(),
            espaco.as_mut_ptr(),
            espaco.len(),
            &mut achado,
        )
    };
    if erro != 0 || achado.is_null() {
        return None;
    }
    // SAFETY: com `achado` não nulo, a chamada preencheu `registro`.
    let registro = unsafe { registro.assume_init() };
    if registro.pw_name.is_null() {
        return None;
    }
    // SAFETY: `pw_name` aponta para uma cadeia terminada em NUL dentro de `espaco`, ainda vivo.
    let nome = unsafe { CStr::from_ptr(registro.pw_name) };
    Some(nome.to_string_lossy().into_owned())
}

/// Os tiques do relógio do `proc_taskinfo` em tempo: nanossegundos no Intel e, no Apple
/// Silicon, unidades do `mach_absolute_time` (125/3 ns cada), que o `mach_timebase_info` diz.
///
/// A `libc` marca o `mach_timebase_info` como obsoleto e indica o crate `mach2`, que declara a
/// mesma função do sistema, também como `unsafe`: trocar só traria uma dependência a mais.
#[allow(deprecated)]
fn de_tiques(tiques: u64) -> Duration {
    static ESCALA: OnceLock<(u32, u32)> = OnceLock::new();
    let &(numer, denom) = ESCALA.get_or_init(|| {
        let mut base = libc::mach_timebase_info { numer: 0, denom: 0 };
        // SAFETY: `base` é local e vive durante a chamada, que só a preenche.
        let erro = unsafe { libc::mach_timebase_info(&mut base) };
        if erro != 0 || base.numer == 0 || base.denom == 0 {
            (1, 1)
        } else {
            (base.numer, base.denom)
        }
    });
    converter(tiques, numer, denom)
}

/// `tiques` × `numer` / `denom` nanossegundos, sem estourar no caminho.
pub(crate) fn converter(tiques: u64, numer: u32, denom: u32) -> Duration {
    let nanos = u128::from(tiques) * u128::from(numer) / u128::from(denom.max(1));
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}
