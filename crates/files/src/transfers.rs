//! As [`Transfers`]: a fila de envios, baixas, cópias e exclusões, com trabalhadores em
//! paralelo, conflitos, retomar, pausar e cancelar.
//!
//! Cada trabalhador abre o **seu** [`Fs`] pelo [`Opener`] de cada ponta e o mantém aberto
//! entre arquivos do mesmo par de pontas; no SFTP sobre uma sessão isso é um canal novo na
//! mesma conexão. A tela lê a lista ([`Transfers::items`]) e a pergunta de conflito
//! ([`Transfers::asking`]) por [`Live`] e responde com [`Transfers::answer`].

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::entry::{Entry, EntryKind};
use crate::estimate::{Clock, Estimator, Eta, Route, SystemClock, Work};
use crate::fs::{Fs, Location, TreeOptions};
use crate::live::Live;
use crate::local::Local;
use crate::path;
use crate::remote::{Opener, error_text};

/// Quantas transferências andam ao mesmo tempo, se ninguém mexer.
pub const PARALELAS_PADRAO: usize = 2;
/// Se a data de modificação da origem vai para o destino.
pub const PRESERVAR_DATA_PADRAO: bool = true;
/// Se as permissões da origem vão para o destino.
pub const PRESERVAR_PERMISSOES_PADRAO: bool = false;

/// O tamanho do bloco lido e gravado de cada vez.
const BUFFER: usize = 128 * 1024;
/// O mínimo de bytes e de tempo entre duas atualizações do progresso.
const PROGRESSO_BYTES: u64 = 64 * 1024;
const PROGRESSO_TEMPO: Duration = Duration::from_millis(100);
/// A constante de tempo da média móvel da velocidade.
const VELOCIDADE_TAU: f64 = 2.0;
/// De quanto em quanto tempo o resumo é recalculado enquanto a leva anda.
const RESUMO_PERIODO: Duration = Duration::from_millis(250);
/// O menor intervalo real entre dois cálculos do resumo, para os relatórios de progresso não
/// o recalcularem a cada trecho.
const RESUMO_INTERVALO_MINIMO: Duration = Duration::from_millis(50);
/// Há quanto tempo sem andar um byte a leva é dada como parada.
const PARADA_APOS: Duration = Duration::from_secs(5);

/// Uma ponta de uma transferência.
#[derive(Clone)]
pub enum Endpoint {
    /// O disco desta máquina.
    Local,
    /// Um servidor, aberto por quem chama o [`Opener`].
    Remote(Opener),
}

impl Endpoint {
    /// A identidade da ponta para a estimativa: 0 é o disco desta máquina e um servidor vale o
    /// endereço do seu [`Opener`].
    fn id(&self) -> u64 {
        match self {
            Endpoint::Local => 0,
            Endpoint::Remote(opener) => Arc::as_ptr(opener).cast::<()>() as usize as u64,
        }
    }

    fn same(&self, other: &Endpoint) -> bool {
        match (self, other) {
            (Endpoint::Local, Endpoint::Local) => true,
            (Endpoint::Remote(a), Endpoint::Remote(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Local => f.write_str("Local"),
            Endpoint::Remote(_) => f.write_str("Remote"),
        }
    }
}

/// O que fazer quando o destino já existe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// Perguntar à tela ([`Transfers::asking`]).
    Ask,
    /// Trocar o arquivo.
    Overwrite,
    /// Continuar de onde o destino parou (só se for menor que a origem; senão troca).
    Resume,
    /// Deixar como está.
    Skip,
    /// Gravar com outro nome: `nome (1).ext`, `nome (2).ext`…
    Rename,
}

/// Em que pé está uma transferência.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferState {
    /// Esperando um trabalhador livre.
    Queued,
    /// Andando.
    Running,
    /// Parada por [`Transfers::pause_all`].
    Paused,
    /// Terminou.
    Done,
    /// Deu erro; a mensagem diz qual. [`Transfers::retry`] tenta de novo.
    Failed(String),
    /// Pulada (o destino existia e a escolha foi pular, ou não era um arquivo).
    Skipped,
    /// Cancelada; o arquivo parcial fica no destino, para retomar.
    Cancelled,
    /// Esperando a resposta de [`Transfers::asking`].
    Asking,
}

impl TransferState {
    /// `true` quando não vai mais mudar sozinha.
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            TransferState::Done
                | TransferState::Failed(_)
                | TransferState::Skipped
                | TransferState::Cancelled
        )
    }
}

/// O que uma transferência faz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransferKind {
    /// Do disco local para um servidor.
    Upload,
    /// De um servidor para o disco local.
    Download,
    /// Entre duas pontas do mesmo tipo (disco para disco, ou servidor para servidor).
    Copy,
    /// Apagar (uma pasta é apagada por dentro).
    Delete,
}

/// Uma linha da fila.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferInfo {
    /// O número que [`Transfers::add`] devolveu.
    pub id: u64,
    /// O que faz.
    pub kind: TransferKind,
    /// A origem (o que se apaga, no [`TransferKind::Delete`]).
    pub from: String,
    /// O destino (vazio no [`TransferKind::Delete`]); muda se o conflito for renomear.
    pub to: String,
    /// O tamanho em bytes, quando já se sabe (numa pasta, só a que foi enviada de uma vez pelo
    /// SFTP: o total de bytes dela).
    pub size: Option<u64>,
    /// Quantos bytes já foram (ou quantos itens já foram apagados).
    pub done: u64,
    /// A velocidade em bytes por segundo (média móvel de 2 s).
    pub speed: f64,
    /// O estado.
    pub state: TransferState,
    /// Um aviso sobre esta linha (por exemplo, que o servidor limita as conexões e a fila
    /// passou a usar menos trabalhadores).
    pub notice: Option<String>,
    /// É uma pasta: o item que lista e enfileira o que há dentro.
    pub is_dir: bool,
    /// O tempo restante desta linha, só enquanto anda (M40).
    pub remaining: Option<Eta>,
}

/// Em que pé está a leva de transferências.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueueState {
    /// Nada por fazer: a leva terminou (ou ainda não houve nenhuma).
    #[default]
    Idle,
    /// Andando.
    Running,
    /// Parada por [`Transfers::pause_all`].
    Paused,
    /// Esperando a resposta de [`Transfers::asking`].
    Asking,
}

/// O resumo da leva, para a barra e o texto em cima da fila (M40).
///
/// A **leva** é o conjunto dos itens enfileirados, ou devolvidos por [`Transfers::retry`], desde
/// a última vez que a fila ficou parada: o que terminou antes não pesa na conta.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransferSummary {
    /// O estado da leva.
    pub state: QueueState,
    /// Os itens de arquivo (e de apagar) da leva, sem os pulados, os cancelados e os que
    /// falharam.
    pub files_total: usize,
    /// Quantos desses já terminaram.
    pub files_done: usize,
    /// Quantos itens da leva falharam.
    pub failed: usize,
    /// A leva tem exclusões: a tela fala em "itens", e não em "arquivos".
    pub deletes: bool,
    /// A soma dos tamanhos que já se conhecem.
    pub bytes_total: u64,
    /// Os bytes que já foram.
    pub bytes_done: u64,
    /// Ainda há uma pasta por listar ou um item sem tamanho: os totais ainda crescem.
    pub counting: bool,
    /// Os bytes por segundo da leva (média móvel de 3 s); 0 fora de [`QueueState::Running`].
    pub speed: f64,
    /// O tempo que falta, quando já se mediu o bastante.
    pub eta: Option<Eta>,
    /// Há quanto tempo nenhum byte anda, quando passa de 5 s.
    pub stalled: Option<Duration>,
    /// O instante desta publicação, no relógio da fila.
    pub at: Duration,
}

/// A pergunta que a tela responde com [`Transfers::answer`].
#[derive(Clone, Debug, PartialEq)]
pub struct ConflictQuestion {
    /// A transferência que espera.
    pub id: u64,
    /// O caminho do destino que já existe.
    pub path: String,
    /// Os dados da origem (tamanho e data).
    pub source: Entry,
    /// Os dados do destino que já existe.
    pub target: Entry,
}

#[derive(Clone)]
struct JobData {
    kind: TransferKind,
    from: Endpoint,
    from_path: String,
    to: Endpoint,
    to_path: String,
    conflict: Conflict,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct Queue {
    pending: VecDeque<u64>,
    jobs: HashMap<u64, JobData>,
    next_id: u64,
    limit: usize,
    workers: usize,
    active: usize,
    apply_all: Option<Conflict>,
    answer: Option<(u64, Conflict)>,
    asking_busy: bool,
    /// Os itens da leva (seção "A leva" do [`TransferSummary`]).
    batch: HashSet<u64>,
}

/// O cronômetro de um item que anda: de onde sai o custo fixo do arquivo.
struct Stopwatch {
    /// O `clock.now()` de quando o item começou a rodar.
    started: Duration,
    /// As esperas que não contam: a pausa e a resposta de um conflito.
    excluded: Duration,
    /// O tempo da fase dos bytes, sem as esperas.
    bytes_time: Duration,
    /// O item já está na fase dos bytes.
    in_bytes: bool,
    /// É uma pasta: não vira amostra de custo.
    is_dir: bool,
}

/// O que o laço do resumo guarda entre uma publicação e outra.
#[derive(Default)]
struct TickState {
    /// A leva anda: o laço acorda sozinho a cada [`RESUMO_PERIODO`].
    active: bool,
    /// Os bytes feitos na última publicação.
    last_bytes: u64,
    /// Desde quando `last_bytes` não muda.
    changed_at: Duration,
    /// A leva andava na publicação anterior.
    was_running: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    cv: Condvar,
    items: Live<Vec<TransferInfo>>,
    asking: Live<Option<ConflictQuestion>>,
    paused: AtomicBool,
    closing: AtomicBool,
    preserve_date: AtomicBool,
    preserve_permissions: AtomicBool,
    clock: Arc<dyn Clock>,
    estimator: Mutex<Estimator>,
    watches: Mutex<HashMap<u64, Stopwatch>>,
    summary: Live<TransferSummary>,
    /// Há o que publicar; os trabalhadores só ligam isto e acordam o `tick_cv`.
    dirty: AtomicBool,
    tick_cv: Condvar,
    tick: Mutex<TickState>,
    /// Uma publicação de cada vez, para uma antiga nunca cobrir uma nova.
    publishing: Mutex<()>,
}

struct Handle {
    shared: Arc<Shared>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shared.closing.store(true, Ordering::SeqCst);
        drop(self.shared.lock());
        self.shared.cv.notify_all();
        self.shared.wake();
    }
}

/// A fila de transferências. É barata de copiar; vive na thread da tela. Soltar a última cópia
/// cancela o que anda e encerra os trabalhadores.
#[derive(Clone)]
pub struct Transfers {
    handle: Arc<Handle>,
}

impl std::fmt::Debug for Transfers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transfers").finish_non_exhaustive()
    }
}

impl Transfers {
    /// Cria a fila, com até `parallel` transferências ao mesmo tempo (mínimo 1).
    pub fn new(parallel: usize) -> Transfers {
        Transfers::with_clock(parallel, Arc::new(SystemClock::new()))
    }

    /// Como [`new`](Self::new), mas medindo o tempo com `clock`: com um
    /// [`ManualClock`](crate::ManualClock), os testes do resumo e da estimativa andam o
    /// relógio à mão.
    pub fn with_clock(parallel: usize, clock: Arc<dyn Clock>) -> Transfers {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                limit: parallel.max(1),
                next_id: 1,
                ..Queue::default()
            }),
            cv: Condvar::new(),
            items: Live::new(Vec::new()),
            asking: Live::new(None),
            paused: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            preserve_date: AtomicBool::new(PRESERVAR_DATA_PADRAO),
            preserve_permissions: AtomicBool::new(PRESERVAR_PERMISSOES_PADRAO),
            clock,
            estimator: Mutex::new(Estimator::new()),
            watches: Mutex::new(HashMap::new()),
            summary: Live::new(TransferSummary::default()),
            dirty: AtomicBool::new(false),
            tick_cv: Condvar::new(),
            tick: Mutex::new(TickState::default()),
            publishing: Mutex::new(()),
        });
        shared.ensure_workers();
        let summarizer = Arc::clone(&shared);
        // Se a thread não subir, o resumo só muda quando alguém chama `refresh_summary`.
        let _ = thread::Builder::new()
            .name("transfers-resumo".to_owned())
            .spawn(move || summarizer.summary_loop());
        Transfers {
            handle: Arc::new(Handle { shared }),
        }
    }

    fn shared(&self) -> &Arc<Shared> {
        &self.handle.shared
    }

    /// Muda quantas transferências andam ao mesmo tempo. Vale para as próximas a começar.
    pub fn set_parallel(&self, parallel: usize) {
        self.shared().lock().limit = parallel.max(1);
        self.shared().ensure_workers();
        self.shared().cv.notify_all();
    }

    /// Liga ou desliga copiar a data de modificação da origem.
    pub fn set_preserve_date(&self, on: bool) {
        self.shared().preserve_date.store(on, Ordering::SeqCst);
    }

    /// Liga ou desliga copiar as permissões da origem.
    pub fn set_preserve_permissions(&self, on: bool) {
        self.shared()
            .preserve_permissions
            .store(on, Ordering::SeqCst);
    }

    /// Enfileira a cópia de `from` para `to`. Os dois são **o caminho do próprio item** (não da
    /// pasta onde ele vai): para copiar `/a/x.txt` para a pasta `/b`, `to` é `/b/x.txt`. Uma
    /// pasta é percorrida por dentro (sem seguir links para pastas) e vira uma pasta mais um
    /// item por arquivo; uma pasta **nova** do disco enviada para o SFTP vira um item só
    /// ([`Fs::upload_tree`], com vários arquivos e escritas em voo). Devolve o número da
    /// transferência.
    pub fn add(&self, from: (Endpoint, String), to: (Endpoint, String), conflict: Conflict) -> u64 {
        self.add_sized(from, to, conflict, None)
    }

    /// Como [`add`](Self::add), com o tamanho que quem enfileira já conhece (da listagem da
    /// pasta, por exemplo). Sem ele a leva fica "contando" até a fila ler o arquivo, e o tempo
    /// restante só aparece depois; com ele, aparece já na primeira leitura. A fila confere o
    /// tamanho de novo ao começar o item, e o que ela lê vale mais que a dica.
    pub fn add_sized(
        &self,
        from: (Endpoint, String),
        to: (Endpoint, String),
        conflict: Conflict,
        size: Option<u64>,
    ) -> u64 {
        let kind = match (&from.0, &to.0) {
            (Endpoint::Local, Endpoint::Remote(_)) => TransferKind::Upload,
            (Endpoint::Remote(_), Endpoint::Local) => TransferKind::Download,
            _ => TransferKind::Copy,
        };
        self.shared().enqueue(
            JobData {
                kind,
                from: from.0,
                from_path: from.1,
                to: to.0,
                to_path: to.1,
                conflict,
                cancel: Arc::new(AtomicBool::new(false)),
            },
            size,
            false,
        )
    }

    /// Enfileira apagar `path` (e, numa pasta, tudo o que há dentro).
    pub fn delete(&self, endpoint: Endpoint, path: &str) -> u64 {
        self.shared().enqueue(
            JobData {
                kind: TransferKind::Delete,
                from: endpoint.clone(),
                from_path: path.to_owned(),
                to: endpoint,
                to_path: String::new(),
                conflict: Conflict::Overwrite,
                cancel: Arc::new(AtomicBool::new(false)),
            },
            None,
            false,
        )
    }

    /// A lista da fila, na ordem em que foi pedida.
    pub fn items(&self) -> Live<Vec<TransferInfo>> {
        self.shared().items.clone()
    }

    /// O resumo da leva (M40): totais, velocidade e tempo restante. É recalculado a cada 250 ms
    /// enquanto a leva anda, e na hora quando algo muda.
    pub fn summary(&self) -> Live<TransferSummary> {
        self.shared().summary.clone()
    }

    /// Recalcula o resumo agora, na thread de quem chama. Os testes usam depois de andar o
    /// [`ManualClock`](crate::ManualClock), e o [`wait_idle`](Self::wait_idle) chama antes de
    /// voltar, para quem lê depois dele ver o resumo final.
    pub fn refresh_summary(&self) {
        self.shared().publish_summary();
    }

    /// A pergunta de conflito que espera resposta, se houver (uma de cada vez).
    pub fn asking(&self) -> Live<Option<ConflictQuestion>> {
        self.shared().asking.clone()
    }

    /// Responde a pergunta de `id`. Com `apply_to_all`, a escolha vale para o resto da fila.
    /// [`Conflict::Ask`] não é uma resposta e é ignorada.
    pub fn answer(&self, id: u64, choice: Conflict, apply_to_all: bool) {
        if choice == Conflict::Ask {
            return;
        }
        let mut queue = self.shared().lock();
        if apply_to_all {
            queue.apply_all = Some(choice);
        }
        queue.answer = Some((id, choice));
        self.shared().cv.notify_all();
    }

    /// Cancela uma transferência (na fila ou andando).
    pub fn cancel(&self, id: u64) {
        let shared = self.shared();
        let mut queue = shared.lock();
        let Some(job) = queue.jobs.get(&id) else {
            return;
        };
        job.cancel.store(true, Ordering::SeqCst);
        if let Some(position) = queue.pending.iter().position(|p| *p == id) {
            queue.pending.remove(position);
            shared.patch(id, |info| info.state = TransferState::Cancelled);
        }
        shared.cv.notify_all();
    }

    /// Cancela tudo o que não terminou.
    pub fn cancel_all(&self) {
        let ids: Vec<u64> = self.shared().items.with_untracked(|items| {
            items
                .iter()
                .filter(|i| !i.state.is_finished())
                .map(|i| i.id)
                .collect()
        });
        for id in ids {
            self.cancel(id);
        }
    }

    /// Para tudo: o que anda espera e nada novo começa.
    pub fn pause_all(&self) {
        let shared = self.shared();
        let queue = shared.lock();
        shared.paused.store(true, Ordering::SeqCst);
        let pending: Vec<u64> = queue.pending.iter().copied().collect();
        shared.items.update(|items| {
            for info in items.iter_mut() {
                if info.state == TransferState::Queued && pending.contains(&info.id) {
                    info.state = TransferState::Paused;
                }
            }
        });
        shared.cv.notify_all();
        shared.wake();
    }

    /// Solta o que [`pause_all`](Self::pause_all) parou.
    pub fn resume_all(&self) {
        let shared = self.shared();
        let queue = shared.lock();
        shared.paused.store(false, Ordering::SeqCst);
        let pending: Vec<u64> = queue.pending.iter().copied().collect();
        shared.items.update(|items| {
            for info in items.iter_mut() {
                if info.state == TransferState::Paused && pending.contains(&info.id) {
                    info.state = TransferState::Queued;
                }
            }
        });
        shared.cv.notify_all();
        shared.wake();
    }

    /// `true` se a fila está parada por [`pause_all`](Self::pause_all).
    pub fn is_paused(&self) -> bool {
        self.shared().paused.load(Ordering::SeqCst)
    }

    /// Põe de novo na fila uma transferência que falhou, foi pulada ou cancelada.
    pub fn retry(&self, id: u64) {
        let shared = self.shared();
        let mut queue = shared.lock();
        let finished = shared.items.with_untracked(|items| {
            items
                .iter()
                .any(|i| i.id == id && i.state.is_finished() && i.state != TransferState::Done)
        });
        let Some(job) = queue.jobs.get_mut(&id) else {
            return;
        };
        if !finished {
            return;
        }
        job.cancel = Arc::new(AtomicBool::new(false));
        if queue.active == 0 && queue.pending.is_empty() {
            queue.apply_all = None;
            queue.batch.clear();
        }
        queue.batch.insert(id);
        queue.pending.push_back(id);
        let paused = shared.paused.load(Ordering::SeqCst);
        shared.patch(id, |info| {
            info.done = 0;
            info.speed = 0.0;
            info.remaining = None;
            info.state = if paused {
                TransferState::Paused
            } else {
                TransferState::Queued
            };
        });
        shared.cv.notify_all();
    }

    /// Tira da lista o que já terminou (inclusive as que falharam).
    pub fn clear_finished(&self) {
        let shared = self.shared();
        let mut queue = shared.lock();
        let mut removed = Vec::new();
        shared.items.update(|items| {
            items.retain(|info| {
                let keep = !info.state.is_finished();
                if !keep {
                    removed.push(info.id);
                }
                keep
            });
        });
        for id in removed {
            queue.jobs.remove(&id);
            queue.batch.remove(&id);
        }
        drop(queue);
        shared.wake();
    }

    /// `true` quando não há nada na fila nem andando (nem esperando resposta).
    pub fn is_idle(&self) -> bool {
        let queue = self.shared().lock();
        queue.pending.is_empty() && queue.active == 0
    }

    /// Espera a fila esvaziar, até `timeout`. Devolve `false` se o tempo acabou antes. Bloqueia
    /// a thread: serve aos testes e ao fechamento do app, não à interface.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let shared = self.shared();
        let deadline = Instant::now() + timeout;
        let mut queue = shared.lock();
        let idle = loop {
            if queue.pending.is_empty() && queue.active == 0 {
                break true;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break false;
            };
            queue = shared
                .cv
                .wait_timeout(queue, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        };
        drop(queue);
        shared.publish_summary();
        idle
    }
}

/// Como terminou o trabalho de um item.
enum Outcome {
    Done,
    Skipped,
    Cancelled,
}

/// O que o resumo junta numa passada pelos itens da leva.
#[derive(Default)]
struct Plan {
    files_total: usize,
    files_done: usize,
    failed: usize,
    deletes: bool,
    bytes_total: u64,
    bytes_done: u64,
    counting: bool,
    /// Algum item da leva ainda não terminou.
    unfinished: bool,
    asking: bool,
    /// Algum item andando já está na fase dos bytes.
    in_bytes: bool,
    /// O que falta de cada item por terminar.
    work: Vec<Work>,
    /// O mesmo, só dos itens andando que dá para estimar sozinhos.
    rows: Vec<(u64, Work)>,
}

/// Como o `pause_point` terminou.
enum PauseOutcome {
    /// A fila não estava parada.
    Continue,
    /// Estava parada e voltou.
    Resumed,
    /// A transferência foi cancelada.
    Cancelled,
}

/// A conexão que um trabalhador mantém aberta para uma ponta.
#[derive(Default)]
struct Slot {
    endpoint: Option<Endpoint>,
    fs: Option<Box<dyn Fs>>,
}

impl Slot {
    fn get(&mut self, endpoint: &Endpoint) -> io::Result<&mut dyn Fs> {
        let same = self.endpoint.as_ref().is_some_and(|e| e.same(endpoint));
        if !same || self.fs.is_none() {
            self.fs = None;
            let fs: Box<dyn Fs> = match endpoint {
                Endpoint::Local => Box::new(Local::new()),
                Endpoint::Remote(opener) => opener()?,
            };
            self.fs = Some(fs);
            self.endpoint = Some(endpoint.clone());
        }
        match self.fs.as_mut() {
            Some(fs) => Ok(fs.as_mut()),
            None => Err(io::Error::other("conexão indisponível")),
        }
    }

    /// O aviso que a conexão deixou, se houver.
    fn notice(&mut self) -> Option<String> {
        self.fs.as_mut().and_then(|fs| fs.take_notice())
    }

    fn reset(&mut self) {
        self.fs = None;
        self.endpoint = None;
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn patch(&self, id: u64, f: impl FnOnce(&mut TransferInfo)) {
        self.items.update(|items| {
            if let Some(info) = items.iter_mut().find(|i| i.id == id) {
                f(info);
            }
        });
        self.wake();
    }

    /// Diz ao laço do resumo que há o que recalcular. Quem chama pode estar com o `queue` preso:
    /// o `tick` é uma folha na ordem dos locks.
    fn wake(&self) {
        self.dirty.store(true, Ordering::SeqCst);
        // Pegar e soltar o lock garante que o laço, se está indo dormir, já viu o `dirty`.
        drop(self.tick.lock().unwrap_or_else(PoisonError::into_inner));
        self.tick_cv.notify_all();
    }

    fn watches(&self) -> MutexGuard<'_, HashMap<u64, Stopwatch>> {
        self.watches.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn estimator(&self) -> MutexGuard<'_, Estimator> {
        self.estimator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Mexe no cronômetro de `id`, se o item anda.
    fn watch<R>(&self, id: u64, f: impl FnOnce(&mut Stopwatch) -> R) -> Option<R> {
        self.watches().get_mut(&id).map(f)
    }

    /// Soma uma espera (pausa ou pergunta) ao que o cronômetro de `id` desconta.
    fn exclude(&self, id: u64, from: Duration) {
        let waited = self.clock.now().saturating_sub(from);
        self.watch(id, |watch| watch.excluded += waited);
    }

    fn ensure_workers(self: &Arc<Self>) {
        let mut queue = self.lock();
        while queue.workers < queue.limit {
            let shared = Arc::clone(self);
            let spawned = thread::Builder::new()
                .name("transfers".to_owned())
                .spawn(move || shared.work());
            if spawned.is_err() {
                break;
            }
            queue.workers += 1;
        }
    }

    fn enqueue(self: &Arc<Self>, job: JobData, size: Option<u64>, is_dir: bool) -> u64 {
        self.enqueue_many(vec![(job, size, is_dir)])[0]
    }

    /// Enfileira vários itens de uma vez, na ordem dada na tabela. As pastas vão para o começo
    /// do que está por fazer (na mesma ordem entre elas): listá-las logo conta cedo os arquivos
    /// que ainda não existem na fila.
    fn enqueue_many(self: &Arc<Self>, jobs: Vec<(JobData, Option<u64>, bool)>) -> Vec<u64> {
        if jobs.is_empty() {
            return Vec::new();
        }
        let mut queue = self.lock();
        if queue.active == 0 && queue.pending.is_empty() {
            queue.apply_all = None;
            queue.batch.clear();
        }
        let paused = self.paused.load(Ordering::SeqCst);
        let mut ids = Vec::with_capacity(jobs.len());
        let mut dirs = Vec::new();
        let mut infos = Vec::with_capacity(jobs.len());
        for (job, size, is_dir) in jobs {
            let id = queue.next_id;
            queue.next_id += 1;
            infos.push(TransferInfo {
                id,
                kind: job.kind,
                from: job.from_path.clone(),
                to: job.to_path.clone(),
                size,
                done: 0,
                speed: 0.0,
                state: if paused {
                    TransferState::Paused
                } else {
                    TransferState::Queued
                },
                notice: None,
                is_dir,
                remaining: None,
            });
            queue.jobs.insert(id, job);
            queue.batch.insert(id);
            if is_dir {
                dirs.push(id);
            } else {
                queue.pending.push_back(id);
            }
            ids.push(id);
        }
        for id in dirs.into_iter().rev() {
            queue.pending.push_front(id);
        }
        self.items.update(|items| items.extend(infos));
        self.cv.notify_all();
        drop(queue);
        self.wake();
        ids
    }

    /// A thread do resumo: recalcula quando algo muda e, com a leva andando, a cada
    /// [`RESUMO_PERIODO`], para a velocidade cair sozinha numa conexão travada.
    fn summary_loop(self: Arc<Self>) {
        let mut last: Option<Instant> = None;
        loop {
            {
                let mut tick = self.tick.lock().unwrap_or_else(PoisonError::into_inner);
                loop {
                    if self.closing.load(Ordering::SeqCst) {
                        return;
                    }
                    if self.dirty.swap(false, Ordering::SeqCst) {
                        break;
                    }
                    if tick.active {
                        let (guard, timeout) = self
                            .tick_cv
                            .wait_timeout(tick, RESUMO_PERIODO)
                            .unwrap_or_else(PoisonError::into_inner);
                        tick = guard;
                        if timeout.timed_out() {
                            break;
                        }
                    } else {
                        tick = self
                            .tick_cv
                            .wait(tick)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                }
            }
            if let Some(last) = last
                && let Some(left) = RESUMO_INTERVALO_MINIMO.checked_sub(last.elapsed())
            {
                thread::sleep(left);
            }
            self.publish_summary();
            last = Some(Instant::now());
        }
    }

    /// Recalcula o resumo da leva e o `remaining` das linhas que andam. A ordem dos locks é
    /// `publishing → queue → items → (estimator | watches | tick)`, e nenhum deles é preso
    /// por quem chama.
    fn publish_summary(&self) {
        let _publishing = self
            .publishing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = self.clock.now();
        // 1. O que só o `queue` sabe: a leva, a rota de cada item e o limite de paralelas.
        let (routes, limit) = {
            let queue = self.lock();
            let routes: HashMap<u64, Route> = queue
                .batch
                .iter()
                .filter_map(|id| queue.jobs.get(id).map(|job| (*id, route(job))))
                .collect();
            (routes, queue.limit)
        };
        let paused = self.paused.load(Ordering::SeqCst);

        // 2. Os totais e o que falta fazer de cada item.
        let plan = self.items.with_untracked(|items| {
            let watches = self.watches();
            let mut plan = Plan::default();
            for info in items {
                let Some(route) = routes.get(&info.id) else {
                    continue;
                };
                plan.deletes |= info.kind == TransferKind::Delete;
                match &info.state {
                    TransferState::Done if !info.is_dir => {
                        plan.files_total += 1;
                        plan.files_done += 1;
                        if info.kind != TransferKind::Delete {
                            let size = info.size.unwrap_or(info.done);
                            plan.bytes_total += size;
                            plan.bytes_done += size;
                        }
                    }
                    TransferState::Failed(_) => plan.failed += 1,
                    TransferState::Queued
                    | TransferState::Running
                    | TransferState::Paused
                    | TransferState::Asking => {
                        plan.unfinished = true;
                        plan.asking |= info.state == TransferState::Asking;
                        if info.is_dir {
                            plan.counting = true;
                            continue;
                        }
                        plan.files_total += 1;
                        let in_bytes = watches.get(&info.id).is_some_and(|w| w.in_bytes);
                        let work = if info.kind == TransferKind::Delete {
                            Work {
                                route: *route,
                                files: 1,
                                bytes: 0,
                            }
                        } else if let Some(size) = info.size {
                            let done = info.done.min(size);
                            plan.bytes_total += size;
                            plan.bytes_done += done;
                            Work {
                                route: *route,
                                files: u64::from(!in_bytes),
                                bytes: if in_bytes { size - done } else { size },
                            }
                        } else {
                            plan.counting = true;
                            Work {
                                route: *route,
                                files: 1,
                                bytes: 0,
                            }
                        };
                        plan.work.push(work);
                        if info.state == TransferState::Running {
                            plan.in_bytes |= in_bytes;
                            if info.kind == TransferKind::Delete || info.size.is_some() {
                                plan.rows.push((info.id, work));
                            }
                        }
                    }
                    _ => {}
                }
            }
            plan
        });
        let state = if !plan.unfinished {
            QueueState::Idle
        } else if paused {
            QueueState::Paused
        } else if plan.asking {
            QueueState::Asking
        } else {
            QueueState::Running
        };
        let running = state == QueueState::Running;

        // A parada: nenhum byte andou, com um item na fase dos bytes, por um tempo.
        let stalled = {
            let mut tick = self.tick.lock().unwrap_or_else(PoisonError::into_inner);
            if !running || !tick.was_running || plan.bytes_done != tick.last_bytes {
                tick.last_bytes = plan.bytes_done;
                tick.changed_at = now;
            }
            tick.was_running = running;
            tick.active = state != QueueState::Idle;
            let quiet = now.saturating_sub(tick.changed_at);
            (running && plan.in_bytes && quiet >= PARADA_APOS).then_some(quiet)
        };

        // 3. A velocidade da fila e as estimativas.
        let (speed, eta, remaining) = {
            let mut estimator = self.estimator();
            if state == QueueState::Idle {
                estimator.reset_queue_speed();
            }
            let speed = estimator.tick(now, plan.bytes_done, running);
            let eta = (running && !plan.counting && stalled.is_none())
                .then(|| estimator.estimate(&plan.work, limit))
                .flatten();
            let remaining: HashMap<u64, Eta> = plan
                .rows
                .iter()
                .filter_map(|(id, work)| Some((*id, estimator.estimate(&[*work], 1)?)))
                .collect();
            (if running { speed } else { 0.0 }, eta, remaining)
        };

        // 4. As linhas (só se algo mudou) e o resumo.
        let changed = self.items.with_untracked(|items| {
            items
                .iter()
                .any(|info| info.remaining != remaining.get(&info.id).copied())
        });
        if changed {
            self.items.update(|items| {
                for info in items.iter_mut() {
                    info.remaining = if info.state == TransferState::Running {
                        remaining.get(&info.id).copied()
                    } else {
                        None
                    };
                }
            });
        }
        self.summary.set(TransferSummary {
            state,
            files_total: plan.files_total,
            files_done: plan.files_done,
            failed: plan.failed,
            deletes: plan.deletes,
            bytes_total: plan.bytes_total,
            bytes_done: plan.bytes_done,
            counting: plan.counting,
            speed,
            eta,
            stalled,
            at: now,
        });
    }

    fn stopped(&self, job: &JobData) -> bool {
        job.cancel.load(Ordering::SeqCst) || self.closing.load(Ordering::SeqCst)
    }

    /// O laço de cada trabalhador: pega o próximo da fila e cuida dele.
    fn work(self: Arc<Self>) {
        let mut from_slot = Slot::default();
        let mut to_slot = Slot::default();
        loop {
            let (id, job) = {
                let mut queue = self.lock();
                loop {
                    if self.closing.load(Ordering::SeqCst) || queue.workers > queue.limit {
                        queue.workers -= 1;
                        return;
                    }
                    if !self.paused.load(Ordering::SeqCst)
                        && queue.active < queue.limit
                        && let Some(id) = queue.pending.pop_front()
                    {
                        if let Some(job) = queue.jobs.get(&id).cloned() {
                            queue.active += 1;
                            break (id, job);
                        }
                        continue;
                    }
                    queue = self.cv.wait(queue).unwrap_or_else(PoisonError::into_inner);
                }
            };
            self.watches().insert(
                id,
                Stopwatch {
                    started: self.clock.now(),
                    excluded: Duration::ZERO,
                    bytes_time: Duration::ZERO,
                    in_bytes: false,
                    is_dir: false,
                },
            );
            self.patch(id, |info| info.state = TransferState::Running);
            let result = self.process(id, &job, &mut from_slot, &mut to_slot);
            self.finish_watch(id, &job, matches!(result, Ok(Outcome::Done)));
            if let Some(text) = from_slot.notice().or_else(|| to_slot.notice()) {
                self.patch(id, |info| info.notice = Some(text));
            }
            let state = match result {
                Ok(Outcome::Done) => TransferState::Done,
                Ok(Outcome::Skipped) => TransferState::Skipped,
                Ok(Outcome::Cancelled) => TransferState::Cancelled,
                Err(error) if crate::fs::is_connection_limit(&error) && self.reduce_limit(id) => {
                    from_slot.reset();
                    to_slot.reset();
                    TransferState::Queued
                }
                Err(error) => {
                    if crate::fs::is_connection_error(&error) {
                        from_slot.reset();
                        to_slot.reset();
                    }
                    TransferState::Failed(error_text(&error))
                }
            };
            self.patch(id, |info| {
                info.speed = 0.0;
                info.remaining = None;
                info.state = state;
            });
            let mut queue = self.lock();
            queue.active -= 1;
            self.cv.notify_all();
        }
    }

    /// Tira o cronômetro de `id`. Um arquivo que terminou bem (e não uma pasta) deixa uma amostra
    /// do custo fixo: o tempo ativo menos o dos bytes.
    fn finish_watch(&self, id: u64, job: &JobData, done: bool) {
        let Some(watch) = self.watches().remove(&id) else {
            return;
        };
        if done && !watch.is_dir {
            let active = self
                .clock
                .now()
                .saturating_sub(watch.started)
                .saturating_sub(watch.excluded);
            let cost = active.saturating_sub(watch.bytes_time).as_secs_f64();
            self.estimator().file_done(route(job), cost);
        }
        self.wake();
    }

    /// O servidor recusou uma conexão nova: usa um trabalhador a menos (até 1), devolve o item
    /// ao começo da fila e avisa na linha. `false` se já era 1: aí a transferência falha.
    fn reduce_limit(&self, id: u64) -> bool {
        let mut queue = self.lock();
        if queue.limit <= 1 {
            return false;
        }
        queue.limit -= 1;
        let limit = queue.limit;
        queue.pending.push_front(id);
        self.patch(id, |info| {
            info.notice = Some(format!(
                "o servidor limita as conexões; usando {limit} por vez"
            ));
        });
        self.cv.notify_all();
        true
    }

    fn process(
        self: &Arc<Self>,
        id: u64,
        job: &JobData,
        from_slot: &mut Slot,
        to_slot: &mut Slot,
    ) -> io::Result<Outcome> {
        if self.stopped(job) {
            return Ok(Outcome::Cancelled);
        }
        match job.kind {
            TransferKind::Delete => {
                let fs = from_slot.get(&job.from)?;
                let mut count = 0;
                let flow = self.delete_tree(fs, &job.from_path, id, job, &mut count)?;
                Ok(flow)
            }
            _ => {
                let from = from_slot.get(&job.from)?;
                let to = to_slot.get(&job.to)?;
                self.transfer(id, job, from, to)
            }
        }
    }

    fn delete_tree(
        &self,
        fs: &mut dyn Fs,
        target: &str,
        id: u64,
        job: &JobData,
        count: &mut u64,
    ) -> io::Result<Outcome> {
        if self.stopped(job) {
            return Ok(Outcome::Cancelled);
        }
        let entry = fs.lstat(target)?;
        if entry.kind == EntryKind::Dir {
            for child in fs.list(target)? {
                let flow =
                    self.delete_tree(fs, &path::join(target, &child.name), id, job, count)?;
                if matches!(flow, Outcome::Cancelled) {
                    return Ok(flow);
                }
            }
            fs.remove_dir(target)?;
        } else {
            fs.remove_file(target)?;
        }
        *count += 1;
        let done = *count;
        self.patch(id, |info| info.done = done);
        Ok(Outcome::Done)
    }

    fn transfer(
        self: &Arc<Self>,
        id: u64,
        job: &JobData,
        from: &mut dyn Fs,
        to: &mut dyn Fs,
    ) -> io::Result<Outcome> {
        let source = from.stat(&job.from_path)?;
        match source.kind {
            EntryKind::Dir => {
                self.patch(id, |info| info.is_dir = true);
                self.watch(id, |watch| watch.is_dir = true);
                if let Some(outcome) = self.upload_tree(id, job, to) {
                    return outcome;
                }
                self.transfer_dir(id, job, from, to)
            }
            EntryKind::File => self.transfer_file(id, job, source, from, to),
            _ => Ok(Outcome::Skipped),
        }
    }

    /// Cria a pasta no destino e enfileira o que há dentro.
    fn transfer_dir(
        self: &Arc<Self>,
        id: u64,
        job: &JobData,
        from: &mut dyn Fs,
        to: &mut dyn Fs,
    ) -> io::Result<Outcome> {
        match to.lstat(&job.to_path) {
            Ok(existing) if existing.kind == EntryKind::Dir => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("já existe e não é uma pasta: {}", job.to_path),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => to.mkdir(&job.to_path)?,
            Err(error) => return Err(error),
        }
        let _ = id;
        let mut children = Vec::new();
        for child in from.list(&job.from_path)? {
            let from_path = path::join(&job.from_path, &child.name);
            let (wanted, size, is_dir) = match child.kind {
                EntryKind::File => (true, Some(child.size), false),
                EntryKind::Dir => (true, None, true),
                // O link para arquivo vira o arquivo; o link para pasta fica de fora (ciclos).
                EntryKind::Symlink => match from.stat(&from_path) {
                    Ok(target) if target.kind == EntryKind::File => {
                        (true, Some(target.size), false)
                    }
                    _ => (false, None, false),
                },
                EntryKind::Other => (false, None, false),
            };
            if wanted {
                let job = JobData {
                    from_path,
                    to_path: path::join(&job.to_path, &child.name),
                    cancel: Arc::clone(&job.cancel),
                    ..job.clone()
                };
                children.push((job, size, is_dir));
            }
        }
        self.enqueue_many(children);
        Ok(Outcome::Done)
    }

    fn transfer_file(
        self: &Arc<Self>,
        id: u64,
        job: &JobData,
        source: Entry,
        from: &mut dyn Fs,
        to: &mut dyn Fs,
    ) -> io::Result<Outcome> {
        self.patch(id, |info| info.size = Some(source.size));
        let mut target = job.to_path.clone();
        let mut offset = 0;
        match to.lstat(&target) {
            Ok(existing) => {
                if existing.kind == EntryKind::Dir {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!("já existe uma pasta com esse nome: {target}"),
                    ));
                }
                let choice = match job.conflict {
                    Conflict::Ask => match self.ask(id, job, &target, &source, &existing) {
                        Some(choice) => choice,
                        None => return Ok(Outcome::Cancelled),
                    },
                    other => other,
                };
                match choice {
                    Conflict::Skip | Conflict::Ask => return Ok(Outcome::Skipped),
                    Conflict::Overwrite => {}
                    Conflict::Resume => {
                        if existing.size < source.size && to.caps().resume_upload {
                            offset = existing.size;
                        }
                    }
                    Conflict::Rename => {
                        target = free_name(to, &target)?;
                        let renamed = target.clone();
                        self.patch(id, |info| info.to = renamed);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // O S3 não deixa um objeto pela metade: o que sobra é um envio em partes, que
                // o próprio servidor conta.
                if job.conflict == Conflict::Resume && to.caps().resume_upload {
                    let pending = to.resume_offset(&target).unwrap_or(0);
                    if pending > 0 && pending < source.size {
                        offset = pending;
                    }
                }
            }
            Err(error) => return Err(error),
        }
        self.patch(id, |info| info.done = offset);

        if offset == 0
            && let Some(flow) = self.delegate(id, job, &target, from, to)
        {
            let flow = flow?;
            if !matches!(flow, Outcome::Cancelled) {
                self.patch(id, |info| info.done = source.size);
            }
            return Ok(flow);
        }

        let flow = {
            let download = from.open_read(&job.from_path, offset)?;
            let modified = source
                .modified
                .filter(|_| self.preserve_date.load(Ordering::SeqCst));
            let upload = to.open_write_with(&target, offset, modified)?;
            self.copy(id, job, download, upload, offset)?
        };
        if matches!(flow, Outcome::Cancelled) {
            return Ok(flow);
        }
        if self.preserve_date.load(Ordering::SeqCst)
            && let Some(time) = source.modified
        {
            let _ = to.set_modified(&target, time);
        }
        if self.preserve_permissions.load(Ordering::SeqCst)
            && let Some(mode) = source.permissions
        {
            let _ = to.set_permissions(&target, mode);
        }
        Ok(Outcome::Done)
    }

    /// O `progress` de uma cópia que o backend faz por conta própria (`delegated_copy`,
    /// `upload_tree`): alimenta a estimativa, o tamanho, o feito e a velocidade do item `id`.
    /// `started` é o instante em que a cópia começou.
    fn bytes_progress<'a>(
        &'a self,
        id: u64,
        job: &JobData,
        started: Duration,
    ) -> impl Fn(u64, u64) + 'a {
        let route = route(job);
        // O instante e os bytes da chamada anterior do `progress`, para a amostra de vazão.
        let previous = Cell::new((started, 0_u64));
        move |done: u64, total: u64| {
            let now = self.clock.now();
            let (before, done_before) = previous.replace((now, done));
            self.watch(id, |watch| watch.in_bytes = true);
            self.estimator().bytes(
                route,
                done.saturating_sub(done_before),
                now.saturating_sub(before).as_secs_f64(),
            );
            let seconds = now.saturating_sub(started).as_secs_f64().max(0.001);
            self.patch(id, |info| {
                info.done = done;
                if total > 0 {
                    info.size = Some(total);
                }
                info.speed = done as f64 / seconds;
            });
        }
    }

    /// Uma pasta do disco que ainda não existe no SFTP vai de uma vez, pelo
    /// [`Fs::upload_tree`]: um item só, com todos os bytes, em vez de um item por arquivo.
    /// `None`: não é o caso (ou o backend não sabe), e a pasta segue item a item.
    fn upload_tree(&self, id: u64, job: &JobData, to: &mut dyn Fs) -> Option<io::Result<Outcome>> {
        if !matches!((&job.from, &job.to), (Endpoint::Local, Endpoint::Remote(_))) {
            return None;
        }
        match to.lstat(&job.to_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return None,
        }
        let options = TreeOptions {
            preserve_date: self.preserve_date.load(Ordering::SeqCst),
            preserve_permissions: self.preserve_permissions.load(Ordering::SeqCst),
            ..TreeOptions::default()
        };
        let started = self.clock.now();
        let progress = self.bytes_progress(id, job, started);
        let excluded_before = self.watch(id, |watch| watch.excluded).unwrap_or_default();
        let result = to.upload_tree(
            Path::new(&job.from_path),
            &job.to_path,
            &options,
            &progress,
            &job.cancel,
        )?;
        let excluded = self
            .watch(id, |watch| watch.excluded)
            .unwrap_or_default()
            .saturating_sub(excluded_before);
        let spent = self.clock.now().saturating_sub(started);
        self.watch(id, |watch| {
            watch.bytes_time += spent.saturating_sub(excluded);
        });
        Some(match result {
            Ok(()) => Ok(Outcome::Done),
            Err(_) if self.stopped(job) => Ok(Outcome::Cancelled),
            Err(error) => Err(error),
        })
    }

    /// Tenta a cópia que o próprio backend faz sem passar os bytes pelo app (o rclone, entre o
    /// disco e a nuvem ou entre duas nuvens dele). `None`: ninguém sabe, copia-se em fluxo.
    fn delegate(
        &self,
        id: u64,
        job: &JobData,
        target: &str,
        from: &mut dyn Fs,
        to: &mut dyn Fs,
    ) -> Option<io::Result<Outcome>> {
        let started = self.clock.now();
        let progress = self.bytes_progress(id, job, started);
        let excluded_before = self.watch(id, |watch| watch.excluded).unwrap_or_default();
        let result = match (&job.from, &job.to) {
            (Endpoint::Local, Endpoint::Remote(_)) => to.delegated_copy(
                &Location::Local(PathBuf::from(&job.from_path)),
                &Location::Here(target.to_owned()),
                &progress,
                &job.cancel,
            ),
            (Endpoint::Remote(_), Endpoint::Local) => from.delegated_copy(
                &Location::Here(job.from_path.clone()),
                &Location::Local(PathBuf::from(target)),
                &progress,
                &job.cancel,
            ),
            (Endpoint::Remote(_), Endpoint::Remote(_)) => {
                let (origem, destino) = (from.delegation()?, to.delegation()?);
                if origem.group != destino.group {
                    return None;
                }
                from.delegated_copy(
                    &Location::Here(job.from_path.clone()),
                    &Location::Peer(destino, target.to_owned()),
                    &progress,
                    &job.cancel,
                )
            }
            (Endpoint::Local, Endpoint::Local) => None,
        }?;
        let excluded = self
            .watch(id, |watch| watch.excluded)
            .unwrap_or_default()
            .saturating_sub(excluded_before);
        let spent = self.clock.now().saturating_sub(started);
        self.watch(id, |watch| {
            watch.bytes_time += spent.saturating_sub(excluded);
        });
        Some(match result {
            Ok(()) => Ok(Outcome::Done),
            Err(_) if self.stopped(job) => Ok(Outcome::Cancelled),
            Err(error) => Err(error),
        })
    }

    /// Copia os bytes, com o progresso, a pausa e o cancelamento.
    fn copy(
        &self,
        id: u64,
        job: &JobData,
        mut download: Box<dyn crate::fs::Download + '_>,
        mut upload: Box<dyn crate::fs::Upload + '_>,
        start: u64,
    ) -> io::Result<Outcome> {
        let route = route(job);
        let mut buffer = vec![0u8; BUFFER];
        let mut done = start;
        let mut reported = start;
        let began = self.clock.now();
        let excluded_before = self
            .watch(id, |watch| {
                watch.in_bytes = true;
                watch.excluded
            })
            .unwrap_or_default();
        let mut last = began;
        let mut speed = 0.0_f64;
        loop {
            match self.pause_point(id, job) {
                PauseOutcome::Cancelled => return Ok(Outcome::Cancelled),
                // O tempo parado não é velocidade: recomeça a medir de agora.
                PauseOutcome::Resumed => {
                    last = self.clock.now();
                    reported = done;
                }
                PauseOutcome::Continue => {}
            }
            let read = match download.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            upload.write_all(&buffer[..read])?;
            done += read as u64;
            let now = self.clock.now();
            let elapsed = now.saturating_sub(last);
            if done - reported >= PROGRESSO_BYTES && elapsed >= PROGRESSO_TEMPO {
                let seconds = elapsed.as_secs_f64();
                let instant = (done - reported) as f64 / seconds;
                let alpha = 1.0 - (-seconds / VELOCIDADE_TAU).exp();
                speed = if speed == 0.0 {
                    instant
                } else {
                    speed + alpha * (instant - speed)
                };
                self.estimator().bytes(route, done - reported, seconds);
                self.patch(id, |info| {
                    info.done = done;
                    info.speed = speed;
                });
                reported = done;
                last = now;
            }
        }
        // O último trecho, que não chegou a um relatório, também é vazão medida: sem ele um
        // arquivo menor que o relatório nunca mediria nada.
        let now = self.clock.now();
        self.estimator().bytes(
            route,
            done - reported,
            now.saturating_sub(last).as_secs_f64(),
        );
        let excluded = self
            .watch(id, |watch| watch.excluded)
            .unwrap_or_default()
            .saturating_sub(excluded_before);
        let spent = now.saturating_sub(began);
        self.watch(id, |watch| {
            watch.bytes_time += spent.saturating_sub(excluded);
        });
        upload.finish()?;
        download.finish()?;
        self.patch(id, |info| info.done = done);
        Ok(Outcome::Done)
    }

    /// Espera enquanto a fila está parada. `Cancelled` se a transferência foi cancelada, e
    /// `Resumed` se esperou e voltou (quem mede velocidade recomeça de agora).
    fn pause_point(&self, id: u64, job: &JobData) -> PauseOutcome {
        if !self.paused.load(Ordering::SeqCst) {
            return if self.stopped(job) {
                PauseOutcome::Cancelled
            } else {
                PauseOutcome::Continue
            };
        }
        self.patch(id, |info| info.state = TransferState::Paused);
        let waiting_since = self.clock.now();
        let mut queue = self.lock();
        while self.paused.load(Ordering::SeqCst) && !self.stopped(job) {
            queue = self.cv.wait(queue).unwrap_or_else(PoisonError::into_inner);
        }
        drop(queue);
        self.exclude(id, waiting_since);
        if self.stopped(job) {
            return PauseOutcome::Cancelled;
        }
        self.patch(id, |info| info.state = TransferState::Running);
        PauseOutcome::Resumed
    }

    /// Pergunta à tela o que fazer com o destino que existe. `None` se cancelaram enquanto
    /// esperava.
    fn ask(
        &self,
        id: u64,
        job: &JobData,
        target: &str,
        source: &Entry,
        existing: &Entry,
    ) -> Option<Conflict> {
        let waiting_since = self.clock.now();
        let mut queue = self.lock();
        // Uma pergunta de cada vez; quem espera pode ser respondido pelo "aplicar a todos".
        loop {
            if self.stopped(job) {
                drop(queue);
                self.exclude(id, waiting_since);
                return None;
            }
            if let Some(choice) = queue.apply_all {
                drop(queue);
                self.exclude(id, waiting_since);
                return Some(choice);
            }
            if !queue.asking_busy {
                break;
            }
            queue = self.cv.wait(queue).unwrap_or_else(PoisonError::into_inner);
        }
        queue.asking_busy = true;
        queue.answer = None;
        self.patch(id, |info| info.state = TransferState::Asking);
        self.asking.set(Some(ConflictQuestion {
            id,
            path: target.to_owned(),
            source: source.clone(),
            target: existing.clone(),
        }));
        let choice = loop {
            if self.stopped(job) {
                break None;
            }
            if let Some((answered, choice)) = queue.answer
                && answered == id
            {
                queue.answer = None;
                break Some(choice);
            }
            queue = self.cv.wait(queue).unwrap_or_else(PoisonError::into_inner);
        };
        queue.asking_busy = false;
        drop(queue);
        self.asking.set(None);
        self.exclude(id, waiting_since);
        self.patch(id, |info| info.state = TransferState::Running);
        self.cv.notify_all();
        choice
    }
}

/// A rota de um item, para a estimativa.
fn route(job: &JobData) -> Route {
    Route {
        kind: job.kind,
        from: job.from.id(),
        to: job.to.id(),
    }
}

/// O primeiro `nome (n).ext` livre ao lado de `target`.
fn free_name(fs: &mut dyn Fs, target: &str) -> io::Result<String> {
    let dir = path::parent(target);
    let name = path::file_name(target).to_owned();
    for n in 1..10_000 {
        let candidate = path::join(&dir, &path::numbered(&name, n));
        match fs.lstat(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("não há nome livre ao lado de {target}"),
    ))
}
