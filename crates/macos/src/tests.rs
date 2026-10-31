use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::cpu::tempos;
use crate::disk::eh_fisico;
use crate::memory::disponivel;
use crate::network::{cabecalhos, interface};
use crate::process::converter;
use crate::{
    CpuTimes, ProcessState, all_pids, child_pids, cpu_times, disk_counters, memory_status,
    network_interfaces, process_info, process_path, process_usage, user_name,
};

#[test]
fn a_conversao_dos_tiques_usa_a_escala() {
    assert_eq!(converter(3, 125, 3), Duration::from_nanos(125));
    assert_eq!(converter(1_000, 1, 1), Duration::from_nanos(1_000));
    assert_eq!(converter(u64::MAX, 125, 3), Duration::from_nanos(u64::MAX));
}

#[test]
fn o_proprio_processo_tem_memoria_e_a_cpu_cresce_com_o_trabalho() {
    let antes = process_usage(std::process::id()).unwrap();
    assert!(antes.memory > 1024 * 1024, "{} bytes", antes.memory);
    // Meio segundo de conta numa thread: a CPU tem de andar perto disso, nem 10 vezes menos
    // (a escala do Apple Silicon esquecida) nem mais do que o tempo de todos os núcleos.
    let inicio = Instant::now();
    let mut x = 0u64;
    while inicio.elapsed() < Duration::from_millis(500) {
        x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
    }
    let gasto = inicio.elapsed();
    let depois = process_usage(std::process::id()).unwrap();
    let cpu = depois.cpu - antes.cpu;
    let nucleos = std::thread::available_parallelism().map_or(1, usize::from) as u32;
    assert!(cpu >= gasto / 2, "{cpu:?} de CPU em {gasto:?}");
    assert!(
        cpu <= gasto * nucleos + Duration::from_millis(200),
        "{cpu:?} em {gasto:?}"
    );
}

#[test]
fn os_filhos_aparecem_e_um_pid_que_nao_existe_falha() {
    let mut filho = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let pid = filho.id();
    let achou = child_pids(std::process::id()).unwrap().contains(&pid);
    let uso = process_usage(pid);
    filho.kill().unwrap();
    filho.wait().unwrap();
    assert!(achou, "o filho {pid} não está na lista");
    assert!(uso.unwrap().memory > 0);
    assert!(child_pids(pid).unwrap_or_default().is_empty());
    assert!(process_usage(u32::MAX).is_err());
    assert!(process_usage(999_999).is_err());
}

#[test]
fn a_lista_tem_o_proprio_processo_com_disco_dono_e_caminho() {
    let eu = std::process::id();
    assert!(all_pids().unwrap().contains(&eu));
    let antes = process_info(eu).unwrap();
    assert_eq!(antes.pid, eu);
    assert!(antes.parent_pid > 0);
    assert!(!antes.name.is_empty());
    assert_eq!(antes.state, ProcessState::Running);
    assert!(antes.usage.unwrap().memory > 0);
    assert!(antes.threads.unwrap() >= 1);
    // O disco anda com uma escrita sincronizada.
    let arquivo = std::env::temp_dir().join(format!("tomni-ui-macos-es-{eu}"));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&arquivo).unwrap();
        f.write_all(&vec![7u8; 512 * 1024]).unwrap();
        f.sync_all().unwrap();
    }
    std::fs::remove_file(&arquivo).unwrap();
    let depois = process_info(eu).unwrap();
    assert!(
        depois.disk.unwrap().written_bytes > antes.disk.unwrap().written_bytes,
        "{antes:?} {depois:?}"
    );
    assert_eq!(depois.start_time, antes.start_time, "o mesmo processo");
    // O sistema devolve o caminho sem os atalhos (`/private/var`, não `/var`).
    let exe = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    assert_eq!(process_path(eu).unwrap(), exe.to_string_lossy());
    assert!(user_name(0).as_deref() == Some("root"));
    assert!(user_name(antes.uid).is_some_and(|n| !n.is_empty()));
    assert!(process_info(999_999).is_err());
    assert!(process_path(999_999).is_err());
}

#[test]
fn a_memoria_da_maquina_tem_total_e_disponivel() {
    let memoria = memory_status().unwrap();
    assert!(memoria.total > 256 * 1024 * 1024, "{memoria:?}");
    assert!(
        memoria.available > 0 && memoria.available <= memoria.total,
        "{memoria:?}"
    );
}

#[test]
fn o_disponivel_nao_passa_de_zero() {
    assert_eq!(disponivel(1 << 30, 1024, 16_384), (1 << 30) - 1024 * 16_384);
    assert_eq!(disponivel(1 << 30, u64::MAX, 16_384), 0);
}

#[test]
fn cada_nucleo_tem_os_tempos_desde_o_boot() {
    let nucleos = cpu_times().unwrap();
    let esperado = std::thread::available_parallelism().unwrap().get();
    assert_eq!(nucleos.len(), esperado);
    assert!(
        nucleos.iter().any(|n| n.idle > Duration::ZERO),
        "{nucleos:?}"
    );
    assert!(
        nucleos.iter().any(|n| n.kernel > Duration::ZERO),
        "{nucleos:?}"
    );
}

#[test]
fn os_tiques_viram_tempo_com_o_nice_no_usuario() {
    // Usuário, sistema, ocioso e nice de dois núcleos, a 100 tiques por segundo.
    let tiques = [100, 50, 700, 20, 10, 5, 300, 0];
    assert_eq!(
        tempos(&tiques, 2, 100),
        [
            CpuTimes {
                user: Duration::from_millis(1200),
                kernel: Duration::from_millis(500),
                idle: Duration::from_secs(7),
            },
            CpuTimes {
                user: Duration::from_millis(100),
                kernel: Duration::from_millis(50),
                idle: Duration::from_secs(3),
            },
        ]
    );
}

#[test]
fn as_interfaces_de_rede_vem_sem_a_de_loopback() {
    let interfaces = network_interfaces().unwrap();
    assert!(!interfaces.is_empty());
    assert!(
        interfaces.iter().all(|i| !i.name.starts_with("lo")),
        "{interfaces:?}"
    );
    assert!(interfaces.iter().any(|i| i.up), "{interfaces:?}");
}

#[test]
fn as_mensagens_do_sysctl_viram_interfaces() {
    let tamanho = size_of::<libc::if_msghdr2>();
    let mut bytes = Vec::new();
    // Uma mensagem de endereço (RTM_NEWADDR), que fica de fora.
    bytes.extend_from_slice(&8u16.to_ne_bytes());
    bytes.extend_from_slice(&[5, libc::RTM_NEWADDR as u8, 0, 0, 0, 0]);
    // A interface: tamanho, versão, tipo, endereços, flags, índice e os contadores.
    let mut mensagem = vec![0u8; tamanho];
    mensagem[..2].copy_from_slice(&(tamanho as u16).to_ne_bytes());
    mensagem[3] = libc::RTM_IFINFO2 as u8;
    mensagem[8..12].copy_from_slice(&(libc::IFF_UP | 0x40).to_ne_bytes());
    mensagem[12..14].copy_from_slice(&4u16.to_ne_bytes());
    // O `ifm_data` começa no byte 32; o tipo é o primeiro e os bytes ficam em 64 e 72 dele.
    mensagem[32] = 0x06;
    mensagem[96..104].copy_from_slice(&1500u64.to_ne_bytes());
    mensagem[104..112].copy_from_slice(&700u64.to_ne_bytes());
    bytes.extend_from_slice(&mensagem);

    let lista = cabecalhos(&bytes);
    assert_eq!(lista.len(), 1);
    let indice = lista[0].ifm_index;
    assert_eq!(indice, 4);
    let en0 = interface("en0".into(), &lista[0]).unwrap();
    assert_eq!(
        (en0.received_bytes, en0.sent_bytes, en0.up, en0.physical),
        (1500, 700, true, true)
    );
    assert!(!interface("awdl0".into(), &lista[0]).unwrap().physical);

    let mut loopback = lista[0];
    loopback.ifm_flags |= libc::IFF_LOOPBACK;
    assert_eq!(interface("lo0".into(), &loopback), None);
}

#[test]
fn os_discos_tem_nome_bsd_e_contadores_que_so_sobem() {
    let antes = disk_counters().unwrap();
    // O disco interno sempre aparece, com o nome `diskN`.
    assert!(!antes.is_empty());
    assert!(
        antes.iter().all(|d| d.name.starts_with("disk")),
        "{antes:?}"
    );
    assert!(antes.iter().any(|d| d.physical), "{antes:?}");
    let arquivo = std::env::temp_dir().join(format!("tomni-ui-macos-disco-{}", std::process::id()));
    std::fs::write(&arquivo, vec![7u8; 1024 * 1024]).unwrap();
    std::fs::remove_file(&arquivo).unwrap();
    let depois = disk_counters().unwrap();
    for d in &depois {
        if let Some(a) = antes.iter().find(|a| a.name == d.name) {
            assert!(d.written_bytes >= a.written_bytes, "{a:?} {d:?}");
            assert!(d.write_count >= a.write_count, "{a:?} {d:?}");
        }
    }
    assert!(
        depois.iter().any(|d| d.read_count + d.write_count > 0),
        "{depois:?}"
    );
}

#[test]
fn imagem_de_disco_nao_e_fisica() {
    assert!(eh_fisico(Some("PCI-Express")));
    assert!(eh_fisico(Some("USB")));
    assert!(eh_fisico(None));
    assert!(!eh_fisico(Some("Virtual Interface")));
}
