//! O tempo de cada núcleo, para o gráfico por núcleo e a linha do tempo de kernel do app.

use std::io;
use std::time::Duration;

use crate::memory::porta_da_maquina;

/// O tempo que um núcleo lógico passou em cada modo desde o boot ([`cpu_times`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    /// No modo usuário, com o dos processos de prioridade baixa (`nice`).
    pub user: Duration,
    /// No kernel.
    pub kernel: Duration,
    /// Ocioso.
    pub idle: Duration,
}

/// `host_processor_info` com `PROCESSOR_CPU_LOAD_INFO`: os tiques de cada núcleo lógico, no
/// ritmo do `sysconf(_SC_CLK_TCK)`.
pub fn cpu_times() -> io::Result<Vec<CpuTimes>> {
    let mut nucleos: libc::natural_t = 0;
    let mut info: libc::processor_info_array_t = std::ptr::null_mut();
    let mut quantos: libc::mach_msg_type_number_t = 0;
    // SAFETY: os três ponteiros são de locais que vivem durante a chamada; o vetor que ela
    // aloca fica com o `Vetor` abaixo, que o solta.
    let erro = unsafe {
        libc::host_processor_info(
            porta_da_maquina(),
            libc::PROCESSOR_CPU_LOAD_INFO,
            &mut nucleos,
            &mut info,
            &mut quantos,
        )
    };
    if erro != libc::KERN_SUCCESS {
        return Err(io::Error::other(format!(
            "host_processor_info falhou ({erro})"
        )));
    }
    let vetor = Vetor(info, quantos);
    // SAFETY: a chamada deu certo: `info` aponta para `quantos` inteiros, que vivem até o
    // `drop` de `vetor`, depois do último uso da fatia.
    let tiques = unsafe { std::slice::from_raw_parts(vetor.0, vetor.1 as usize) };
    // SAFETY: o `sysconf` só lê uma configuração do sistema; o nome é uma constante válida.
    let por_segundo = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let por_segundo = u64::try_from(por_segundo)
        .ok()
        .filter(|&p| p > 0)
        .ok_or_else(io::Error::last_os_error)?;
    Ok(tempos(tiques, nucleos as usize, por_segundo))
}

/// Os tempos de `nucleos` núcleos a partir dos tiques do `PROCESSOR_CPU_LOAD_INFO`: quatro por
/// núcleo (usuário, sistema, ocioso e `nice`), `por_segundo` tiques por segundo.
pub(crate) fn tempos(
    tiques: &[libc::integer_t],
    nucleos: usize,
    por_segundo: u64,
) -> Vec<CpuTimes> {
    let estados = libc::CPU_STATE_MAX as usize;
    let duracao = |t: libc::integer_t| {
        // Os tiques são `natural_t` (sem sinal) no sistema.
        Duration::from_nanos(u64::from(t as u32) * 1_000_000_000 / por_segundo)
    };
    tiques
        .chunks_exact(estados)
        .take(nucleos)
        .map(|n| CpuTimes {
            user: duracao(n[libc::CPU_STATE_USER as usize])
                + duracao(n[libc::CPU_STATE_NICE as usize]),
            kernel: duracao(n[libc::CPU_STATE_SYSTEM as usize]),
            idle: duracao(n[libc::CPU_STATE_IDLE as usize]),
        })
        .collect()
}

/// O vetor do `host_processor_info`, que se solta com o `vm_deallocate`.
struct Vetor(libc::processor_info_array_t, libc::mach_msg_type_number_t);

impl Drop for Vetor {
    fn drop(&mut self) {
        let tamanho = self.1 as usize * size_of::<libc::integer_t>();
        let tarefa = tarefa_do_processo();
        // SAFETY: o vetor veio de um `host_processor_info` que deu certo, tem `tamanho` bytes e
        // só é solto aqui; o resultado é ignorado porque não há o que fazer num `drop`.
        let _ = unsafe { libc::vm_deallocate(tarefa, self.0 as libc::vm_address_t, tamanho) };
    }
}

/// A porta da tarefa do processo, dona da memória que o `host_processor_info` aloca.
///
/// A `libc` marca o `mach_task_self` como obsoleto e indica o crate `mach2`, que lê a mesma
/// variável do sistema: trocar só traria uma dependência a mais.
#[allow(deprecated)]
fn tarefa_do_processo() -> libc::mach_port_t {
    // SAFETY: o `mach_task_self` só lê a variável que o sistema preenche ao iniciar o processo.
    unsafe { libc::mach_task_self() }
}
