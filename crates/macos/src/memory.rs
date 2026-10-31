//! A memória da máquina, para a lista de processos do app.

use std::io;
use std::sync::OnceLock;

/// A memória física da máquina ([`memory_status`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryStatus {
    /// A memória física instalada, em bytes.
    pub total: u64,
    /// A que os programas podem usar sem o sistema tirar a de outros, em bytes: o total menos a
    /// memória do app (a anônima que não é descartável), a fixa e a comprimida, como o
    /// Monitor de Atividade faz.
    pub available: u64,
}

/// O total pelo `sysctlbyname("hw.memsize")` e o disponível pelo `host_statistics64`
/// (`HOST_VM_INFO64`), com as páginas no tamanho do `sysconf(_SC_PAGESIZE)`.
pub fn memory_status() -> io::Result<MemoryStatus> {
    let total = memoria_instalada()?;
    let vm = estatisticas_da_vm()?;
    // SAFETY: o `sysconf` só lê uma configuração do sistema; o nome é uma constante válida.
    let pagina = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let pagina = u64::try_from(pagina)
        .ok()
        .filter(|&p| p > 0)
        .ok_or_else(io::Error::last_os_error)?;
    let usadas = u64::from(vm.internal_page_count)
        .saturating_sub(u64::from(vm.purgeable_count))
        .saturating_add(u64::from(vm.wire_count))
        .saturating_add(u64::from(vm.compressor_page_count));
    Ok(MemoryStatus {
        total,
        available: disponivel(total, usadas, pagina),
    })
}

/// `total` menos as `usadas` páginas de `pagina` bytes, sem passar de zero.
pub(crate) fn disponivel(total: u64, usadas: u64, pagina: u64) -> u64 {
    total.saturating_sub(usadas.saturating_mul(pagina))
}

/// `hw.memsize`: a memória instalada, em bytes.
fn memoria_instalada() -> io::Result<u64> {
    let mut total = 0u64;
    let mut tamanho = size_of::<u64>();
    // SAFETY: o nome termina em zero; `total` e `tamanho` são locais e vivem durante a chamada,
    // e `tamanho` diz quantos bytes ela pode escrever em `total`; não há valor novo a gravar.
    let erro = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&raw mut total).cast(),
            &mut tamanho,
            std::ptr::null_mut(),
            0,
        )
    };
    if erro != 0 {
        return Err(io::Error::last_os_error());
    }
    if tamanho != size_of::<u64>() || total == 0 {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
    }
    Ok(total)
}

/// `host_statistics64` com `HOST_VM_INFO64`: as contagens de páginas da máquina.
fn estatisticas_da_vm() -> io::Result<libc::vm_statistics64> {
    let mut info = std::mem::MaybeUninit::<libc::vm_statistics64>::zeroed();
    let mut quantos = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: `info` é local, vive durante a chamada e tem os `quantos` inteiros que ela pode
    // escrever; um sistema mais velho escreve menos e diminui `quantos`.
    let erro = unsafe {
        libc::host_statistics64(
            porta_da_maquina(),
            libc::HOST_VM_INFO64,
            info.as_mut_ptr().cast(),
            &mut quantos,
        )
    };
    if erro != libc::KERN_SUCCESS {
        return Err(io::Error::other(format!(
            "host_statistics64 falhou ({erro})"
        )));
    }
    // SAFETY: a chamada deu certo e escreveu o começo da estrutura; o resto continua com os
    // zeros, que já eram válidos (só inteiros).
    Ok(unsafe { info.assume_init() })
}

/// A porta da máquina para o `host_statistics64` e o `host_processor_info`, pedida uma vez só
/// (cada `mach_host_self` soma uma referência à porta).
///
/// A `libc` marca o `mach_host_self` como obsoleto e indica o crate `mach2`, que declara a
/// mesma função do sistema: trocar só traria uma dependência a mais.
#[allow(deprecated)]
pub(crate) fn porta_da_maquina() -> libc::host_t {
    static PORTA: OnceLock<libc::host_t> = OnceLock::new();
    // SAFETY: o `mach_host_self` não recebe nada e só devolve a porta do processo.
    *PORTA.get_or_init(|| unsafe { libc::mach_host_self() })
}
