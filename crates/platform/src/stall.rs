//! O registro de travamentos ([`WindowOptions::stall_log`](crate::WindowOptions::stall_log)):
//! uma thread vigia olha em que etapa a thread da interface está ([`tomni_ui_core::stages`]) e
//! escreve num arquivo quando ela para e quando ela gasta CPU demais.
//!
//! - **Parada**: a mesma etapa de fora (um evento, o quadro, o desenho) passa de [`STALL`]. A
//!   linha diz a etapa de dentro onde ela está, o estado da thread no sistema (rodando ou
//!   esperando o disco), a carga, a memória, o swap e as threads do app que mais gastaram CPU;
//!   repete a cada [`AGAIN`] enquanto durar, e diz quanto durou quando passa.
//! - **Peso**: a cada [`SUMMARY_EVERY`], se a thread da interface gastou mais de [`BUSY_CPU`]
//!   de uma CPU, ou uma etapa passou de [`SLOW`], o tempo por etapa.
//! - **Ritmo**: o aviso do `Pacer` (a janela que pinta sem parar) também vai para o arquivo.
//!
//! O estado das threads, a carga e a memória vêm do `/proc` e só aparecem no Linux.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tomni_ui_core::stages::{self, Current, Totals};

/// A etapa de fora que passa disto é uma parada.
pub(crate) const STALL: Duration = Duration::from_secs(2);
/// Parada que continua: uma linha a cada tanto.
pub(crate) const AGAIN: Duration = Duration::from_secs(10);
/// De quanto em quanto tempo sai o resumo do peso.
pub(crate) const SUMMARY_EVERY: Duration = Duration::from_secs(60);
/// A fração de uma CPU que a thread da interface pode gastar sem ir para o resumo.
pub(crate) const BUSY_CPU: f64 = 0.3;
/// Uma etapa de fora que passa disto vai para o resumo, mesmo com pouca CPU.
pub(crate) const SLOW: Duration = Duration::from_millis(500);
/// De quanto em quanto tempo a vigia olha.
const CHECK: Duration = Duration::from_millis(250);
/// O arquivo passa disto e recomeça.
const LOG_LIMIT: u64 = 1024 * 1024;
/// Quantas threads entram na lista das mais ocupadas, e quantas etapas no resumo.
const TOP: usize = 6;

static LOG: OnceLock<PathBuf> = OnceLock::new();

/// Liga as etapas e a vigia, gravando em `log`. Só a primeira chamada vale.
pub(crate) fn start(log: PathBuf) {
    if LOG.set(log).is_err() {
        return;
    }
    stages::enable();
    let spawned = std::thread::Builder::new()
        .name("tomni-ui-vigia".into())
        .spawn(|| {
            let probe = Proc;
            let mut watch = Watch::new(Instant::now(), &probe);
            loop {
                std::thread::sleep(CHECK);
                for entry in watch.check(Instant::now(), stages::current(), &probe) {
                    note(&entry);
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("tomni-ui: a vigia dos travamentos não começou: {e}");
    }
}

/// Acrescenta `text` ao registro, se ele estiver ligado.
pub(crate) fn note(text: &str) {
    let Some(log) = LOG.get() else {
        return;
    };
    if let Some(dir) = log.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let full = fs::metadata(log).is_ok_and(|m| m.len() > LOG_LIMIT);
    let written = OpenOptions::new()
        .create(true)
        .append(!full)
        .write(true)
        .truncate(full)
        .open(log)
        .and_then(|mut file| writeln!(file, "--- {}\n{text}\n", jiff::Timestamp::now()));
    if let Err(e) = written {
        eprintln!(
            "tomni-ui: o registro de travamentos não foi gravado em {}: {e}",
            log.display()
        );
    }
}

/// Uma thread do processo: o nome, o estado e o tempo de CPU somado, em centésimos de segundo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Task {
    pub(crate) name: String,
    pub(crate) state: char,
    pub(crate) ticks: u64,
    /// Onde ela dorme no kernel (`wchan`), quando o sistema diz.
    pub(crate) wchan: Option<String>,
}

/// O que a vigia sabe do sistema.
pub(crate) trait Probe {
    /// As threads do processo, pelo id. A principal tem o id do processo.
    fn tasks(&self) -> HashMap<u32, Task>;
    /// A carga, a memória do app e a do sistema, numa linha.
    fn system(&self) -> Option<String>;
}

/// O `/proc` do Linux; vazio nos outros sistemas.
struct Proc;

impl Probe for Proc {
    fn tasks(&self) -> HashMap<u32, Task> {
        let mut tasks = HashMap::new();
        let Ok(dir) = fs::read_dir("/proc/self/task") else {
            return tasks;
        };
        for entry in dir.flatten() {
            let Some(tid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
                continue;
            };
            let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            let Some(mut task) = parse_stat(&stat) else {
                continue;
            };
            task.wchan = fs::read_to_string(entry.path().join("wchan"))
                .ok()
                .map(|w| w.trim().to_owned())
                .filter(|w| !w.is_empty() && w != "0");
            tasks.insert(tid, task);
        }
        tasks
    }

    fn system(&self) -> Option<String> {
        let load = fs::read_to_string("/proc/loadavg").ok()?;
        let load: Vec<_> = load.split_whitespace().take(3).collect();
        let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
        let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let kib = |text: &str, key: &str| -> Option<u64> {
            text.lines()
                .find_map(|line| line.strip_prefix(key))
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|n| n.parse().ok())
        };
        let mut line = format!("carga {}", load.join(" ").replace('.', ","));
        if let Some(rss) = kib(&status, "VmRSS:") {
            let _ = write!(line, "; memória do app {}", size(rss));
        }
        if let Some(available) = kib(&meminfo, "MemAvailable:") {
            let _ = write!(line, "; memória disponível {}", size(available));
        }
        if let (Some(total), Some(free)) = (kib(&meminfo, "SwapTotal:"), kib(&meminfo, "SwapFree:"))
            && total > 0
        {
            let _ = write!(
                line,
                "; swap {} de {} em uso",
                size(total - free),
                size(total)
            );
        }
        Some(line)
    }
}

/// O `/proc/<pid>/task/<tid>/stat`: o nome entre parênteses, depois o estado e, nos campos 14
/// e 15, o tempo de CPU do usuário e do sistema.
pub(crate) fn parse_stat(stat: &str) -> Option<Task> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_owned();
    let fields: Vec<_> = stat.get(close + 1..)?.split_whitespace().collect();
    let state = fields.first()?.chars().next()?;
    let user: u64 = fields.get(11)?.parse().ok()?;
    let system: u64 = fields.get(12)?.parse().ok()?;
    Some(Task {
        name,
        state,
        ticks: user + system,
        wchan: None,
    })
}

/// "742 MB", "1,5 GB", de KiB.
fn size(kib: u64) -> String {
    let mb = kib as f64 / 1024.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0).replace('.', ",")
    } else {
        format!("{mb:.0} MB")
    }
}

/// "350 ms", "2,1 s".
pub(crate) fn duration(d: Duration) -> String {
    if d < Duration::from_secs(1) {
        format!("{} ms", d.as_millis())
    } else {
        format!("{:.1} s", d.as_secs_f64()).replace('.', ",")
    }
}

fn state_name(state: char) -> &'static str {
    match state {
        'R' => "rodando",
        'S' => "dormindo",
        'D' => "esperando o disco ou um dispositivo",
        'T' | 't' => "parada pelo sistema",
        'Z' => "encerrada",
        _ => "em outro estado",
    }
}

/// A fração de uma CPU que cada thread gastou entre duas amostras, da que mais gastou.
fn busiest(
    before: &HashMap<u32, Task>,
    after: &HashMap<u32, Task>,
    span: Duration,
) -> Vec<(String, f64)> {
    let ticks = span.as_secs_f64() * 100.0;
    if ticks <= 0.0 {
        return Vec::new();
    }
    let mut list: Vec<_> = after
        .iter()
        .filter_map(|(tid, task)| {
            let previous = before.get(tid).map_or(0, |t| t.ticks);
            let used = task.ticks.saturating_sub(previous) as f64 / ticks;
            (used >= 0.05).then(|| (format!("{} ({tid})", task.name), used))
        })
        .collect();
    list.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    list.truncate(TOP);
    list
}

fn percent(fraction: f64) -> String {
    format!("{:.0}%", fraction * 100.0)
}

/// A vigia: o que já disse e as amostras para comparar.
pub(crate) struct Watch {
    /// A parada já escrita: a etapa de fora (`epoch`), quanto durava da última vez que a vigia
    /// viu, a etapa de dentro e quando foi a última linha.
    reported: Option<(u64, Duration, String, Instant)>,
    /// A amostra das threads de antes, para a CPU de cada uma até agora.
    sample: (Instant, HashMap<u32, Task>),
    /// A amostra do começo do minuto, para o resumo.
    minute: (Instant, HashMap<u32, Task>),
}

impl Watch {
    pub(crate) fn new(now: Instant, probe: &impl Probe) -> Self {
        let tasks = probe.tasks();
        Self {
            reported: None,
            sample: (now, tasks.clone()),
            minute: (now, tasks),
        }
    }

    /// Olha a etapa `current` em `now` e devolve as entradas a gravar.
    pub(crate) fn check(
        &mut self,
        now: Instant,
        current: Option<Current>,
        probe: &impl Probe,
    ) -> Vec<String> {
        let mut entries = Vec::new();
        let stuck = current.filter(|c| c.elapsed >= STALL);
        // A parada escrita acabou: outra etapa de fora começou, ou nenhuma.
        if let Some((epoch, elapsed, path, _)) = &self.reported
            && stuck.as_ref().is_none_or(|c| c.epoch != *epoch)
        {
            entries.push(format!(
                "a interface voltou depois de {} parada (a última etapa vista: {path})",
                duration(*elapsed)
            ));
            self.reported = None;
        }
        match (stuck, &mut self.reported) {
            (Some(c), Some((_, elapsed, path, last))) => {
                *elapsed = c.elapsed;
                path.clone_from(&c.path);
                if now.saturating_duration_since(*last) >= AGAIN {
                    *last = now;
                    entries.push(self.stalled("continua parada", &c, now, probe));
                }
            }
            (Some(c), None) => {
                entries.push(self.stalled("a interface está parada", &c, now, probe));
                self.reported = Some((c.epoch, c.elapsed, c.path, now));
            }
            (None, _) => {
                // Fora de uma parada, a amostra anda: a CPU da próxima parada conta desde aqui.
                if now.saturating_duration_since(self.sample.0) >= STALL {
                    self.sample = (now, probe.tasks());
                }
            }
        }
        if now.saturating_duration_since(self.minute.0) >= SUMMARY_EVERY {
            let totals = stages::take_totals();
            let tasks = probe.tasks();
            let span = now.saturating_duration_since(self.minute.0);
            if let Some(entry) = summary(&totals, &self.minute.1, &tasks, span, probe) {
                entries.push(entry);
            }
            self.minute = (now, tasks);
        }
        entries
    }

    /// A linha da parada, com a thread da interface, o sistema e as threads mais ocupadas
    /// desde a última amostra.
    fn stalled(&self, what: &str, c: &Current, now: Instant, probe: &impl Probe) -> String {
        let mut entry = format!("{what} há {} em {}", duration(c.elapsed), c.path);
        let tasks = probe.tasks();
        let span = now.saturating_duration_since(self.sample.0);
        if let Some(main) = tasks.get(&std::process::id()) {
            let previous = self
                .sample
                .1
                .get(&std::process::id())
                .map_or(main.ticks, |t| t.ticks);
            let used =
                main.ticks.saturating_sub(previous) as f64 / (span.as_secs_f64() * 100.0).max(1.0);
            let _ = write!(
                entry,
                "\n  thread da interface: {} ({}), {} de uma CPU nos últimos {}",
                state_name(main.state),
                main.state,
                percent(used),
                duration(span)
            );
            if let Some(wchan) = &main.wchan {
                let _ = write!(entry, ", no kernel em {wchan}");
            }
        }
        if let Some(system) = probe.system() {
            let _ = write!(entry, "\n  sistema: {system}");
        }
        let busy = busiest(&self.sample.1, &tasks, span);
        if !busy.is_empty() {
            let list: Vec<_> = busy
                .iter()
                .map(|(name, used)| format!("{name} {}", percent(*used)))
                .collect();
            let _ = write!(entry, "\n  threads mais ocupadas: {}", list.join(", "));
        }
        entry
    }
}

/// O resumo do minuto, se a thread da interface pesou: CPU acima de [`BUSY_CPU`] ou uma etapa
/// acima de [`SLOW`].
fn summary(
    totals: &Totals,
    before: &HashMap<u32, Task>,
    after: &HashMap<u32, Task>,
    span: Duration,
    probe: &impl Probe,
) -> Option<String> {
    let pid = std::process::id();
    let cpu = after.get(&pid).map(|main| {
        let previous = before.get(&pid).map_or(0, |t| t.ticks);
        main.ticks.saturating_sub(previous) as f64 / (span.as_secs_f64() * 100.0).max(1.0)
    });
    let slow = totals.slowest.as_ref().filter(|(_, took)| *took >= SLOW);
    if cpu.is_none_or(|cpu| cpu < BUSY_CPU) && slow.is_none() {
        return None;
    }
    let mut entry = format!("no último {}", duration(span));
    match cpu {
        Some(cpu) => {
            let _ = write!(
                entry,
                ", a thread da interface usou {} de uma CPU",
                percent(cpu)
            );
        }
        None => entry.push_str(", a thread da interface pesou"),
    }
    if let Some((name, took)) = &totals.slowest {
        let _ = write!(
            entry,
            "; a etapa mais demorada: {name} ({})",
            duration(*took)
        );
    }
    if !totals.stages.is_empty() {
        let list: Vec<_> = totals
            .stages
            .iter()
            .take(TOP)
            .map(|(name, time, count)| format!("{name} {} em {count}×", duration(*time)))
            .collect();
        let _ = write!(entry, "\n  tempo por etapa: {}", list.join("; "));
    }
    if let Some(system) = probe.system() {
        let _ = write!(entry, "\n  sistema: {system}");
    }
    let busy = busiest(before, after, span);
    if !busy.is_empty() {
        let list: Vec<_> = busy
            .iter()
            .map(|(name, used)| format!("{name} {}", percent(*used)))
            .collect();
        let _ = write!(entry, "\n  threads mais ocupadas: {}", list.join(", "));
    }
    Some(entry)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// Um sistema de mentira: as threads que o teste disser.
    struct Falso(RefCell<HashMap<u32, Task>>);

    impl Falso {
        fn novo() -> Self {
            Self(RefCell::new(HashMap::new()))
        }

        fn thread(&self, tid: u32, nome: &str, estado: char, ticks: u64) {
            self.0.borrow_mut().insert(
                tid,
                Task {
                    name: nome.into(),
                    state: estado,
                    ticks,
                    wchan: None,
                },
            );
        }
    }

    impl Probe for Falso {
        fn tasks(&self) -> HashMap<u32, Task> {
            self.0.borrow().clone()
        }

        fn system(&self) -> Option<String> {
            Some("carga 3,5 6,6 12,2".into())
        }
    }

    fn atual(caminho: &str, segundos: f64, epoch: u64) -> Option<Current> {
        Some(Current {
            path: caminho.into(),
            elapsed: Duration::from_secs_f64(segundos),
            epoch,
        })
    }

    #[test]
    fn le_o_stat_com_parenteses_no_nome() {
        let stat = "1234 (tomni (ui)) R 1 2 3 4 5 6 7 8 9 10 150 25 0 0 20 0 1 0";
        let task = parse_stat(stat).unwrap();
        assert_eq!(task.name, "tomni (ui)");
        assert_eq!(task.state, 'R');
        assert_eq!(task.ticks, 175);
    }

    #[test]
    fn parada_escreve_ao_passar_do_limite_repete_e_diz_quando_voltou() {
        let pid = std::process::id();
        let falso = Falso::novo();
        falso.thread(pid, "tirech-omni", 'R', 0);
        falso.thread(pid + 1, "ssh", 'S', 0);
        let t0 = Instant::now();
        let mut vigia = Watch::new(t0, &falso);
        assert!(vigia.check(t0, atual("quadro", 0.5, 1), &falso).is_empty());
        // Dois segundos depois, a interface gastou 2 s de CPU e o ssh, 1 s.
        falso.thread(pid, "tirech-omni", 'R', 200);
        falso.thread(pid + 1, "ssh", 'S', 100);
        let t1 = t0 + Duration::from_secs(2);
        let linhas = vigia.check(t1, atual("quadro › layout", 2.1, 1), &falso);
        assert_eq!(linhas.len(), 1, "{linhas:?}");
        let linha = &linhas[0];
        assert!(
            linha.starts_with("a interface está parada há 2,1 s em quadro › layout"),
            "{linha}"
        );
        assert!(
            linha.contains("thread da interface: rodando (R), 100% de uma CPU"),
            "{linha}"
        );
        assert!(linha.contains("sistema: carga 3,5"), "{linha}");
        assert!(linha.contains(&format!("ssh ({}) 50%", pid + 1)), "{linha}");
        // Na mesma parada, só depois de `AGAIN`.
        let t2 = t1 + Duration::from_secs(1);
        assert!(
            vigia
                .check(t2, atual("quadro › layout", 3.1, 1), &falso)
                .is_empty()
        );
        let t3 = t1 + AGAIN;
        let linhas = vigia.check(t3, atual("desenho › textura", 12.1, 1), &falso);
        assert!(
            linhas[0].starts_with("continua parada há 12,1 s em desenho › textura"),
            "{linhas:?}"
        );
        // Outra etapa de fora começou: a parada acabou.
        let linhas = vigia.check(t3 + CHECK, atual("evento", 0.01, 2), &falso);
        assert_eq!(
            linhas,
            [
                "a interface voltou depois de 12,1 s parada (a última etapa vista: desenho › textura)"
            ]
        );
        assert!(vigia.check(t3 + 2 * CHECK, None, &falso).is_empty());
    }

    #[test]
    fn o_resumo_sai_so_com_peso() {
        let pid = std::process::id();
        let antes = HashMap::from([(
            pid,
            Task {
                name: "tirech-omni".into(),
                state: 'S',
                ticks: 0,
                wchan: None,
            },
        )]);
        let mut depois = antes.clone();
        depois.get_mut(&pid).unwrap().ticks = 1200;
        let minuto = Duration::from_secs(60);
        let leve = Totals {
            stages: vec![("quadro".into(), Duration::from_secs(1), 600)],
            slowest: Some(("quadro".into(), Duration::from_millis(20))),
        };
        let falso = Falso::novo();
        assert!(
            summary(&leve, &antes, &depois, minuto, &falso).is_none(),
            "20% e quadros curtos"
        );
        depois.get_mut(&pid).unwrap().ticks = 3480;
        let pesado = Totals {
            stages: vec![
                ("quadro › layout".into(), Duration::from_secs(20), 3600),
                ("quadro".into(), Duration::from_secs(5), 3600),
            ],
            slowest: Some(("desenho".into(), Duration::from_millis(1300))),
        };
        let linha = summary(&pesado, &antes, &depois, minuto, &falso).unwrap();
        assert!(
            linha.starts_with(
                "no último 60,0 s, a thread da interface usou 58% de uma CPU; \
                 a etapa mais demorada: desenho (1,3 s)"
            ),
            "{linha}"
        );
        assert!(
            linha.contains(
                "tempo por etapa: quadro › layout 20,0 s em 3600×; quadro 5,0 s em 3600×"
            ),
            "{linha}"
        );
        depois.get_mut(&pid).unwrap().ticks = 0;
        assert!(
            summary(&pesado, &antes, &depois, minuto, &falso).is_some(),
            "pouca CPU, mas uma etapa lenta"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn o_proc_ve_a_thread_principal_e_o_sistema() {
        let threads = Proc.tasks();
        assert!(threads.contains_key(&std::process::id()), "{threads:?}");
        let sistema = Proc.system().unwrap();
        assert!(sistema.starts_with("carga "), "{sistema}");
        assert!(sistema.contains("memória do app"), "{sistema}");
    }

    #[test]
    fn duracoes_em_portugues() {
        assert_eq!(duration(Duration::from_millis(350)), "350 ms");
        assert_eq!(duration(Duration::from_millis(2140)), "2,1 s");
        assert_eq!(size(760_000), "742 MB");
        assert_eq!(size(1_572_864), "1,5 GB");
    }
}
