use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::process::converter;
use crate::{
    ProcessState, all_pids, child_pids, process_info, process_path, process_usage, user_name,
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
