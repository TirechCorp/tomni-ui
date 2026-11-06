//! Executor de frame e escopos de dono.
//!
//! O `reactive_graph` agenda cada efeito como uma tarefa via `any_spawner`. Aqui essas tarefas
//! vão para uma fila por thread que só anda quando o loop de frame (ou o `TestApp`) chama
//! [`run_effects`]: nada roda dentro de um `set`, e várias mudanças no mesmo frame viram uma
//! execução só.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::future::Future;
use std::panic::Location;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Wake, Waker};
use std::time::{Duration, Instant};

use any_spawner::{CustomExecutor, Executor, PinnedFuture, PinnedLocalFuture};
use reactive_graph::owner::Owner;

type Task = Pin<Box<dyn Future<Output = ()>>>;

/// Posição na tabela e geração: um `Waker` antigo não acorda a tarefa que reaproveitou a posição.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TaskId {
    index: usize,
    generation: u64,
}

/// Tarefas acordadas, em ordem. É `Arc<Mutex>` porque o `Waker` precisa ser `Send + Sync`: uma
/// tarefa assíncrona pode ser acordada de outra thread.
type ReadyQueue = Arc<Mutex<VecDeque<TaskId>>>;

#[derive(Default)]
struct Slot {
    generation: u64,
    /// `None` enquanto a tarefa está sendo executada ou depois que terminou.
    task: Option<Task>,
    /// De onde veio o efeito da tarefa, para a medição ([`profile_effects`]).
    origin: Option<EffectOrigin>,
}

#[derive(Default)]
struct Tasks {
    slots: Vec<Slot>,
    /// Posições de tarefas terminadas. A de uma tarefa em execução nunca entra aqui.
    free: Vec<usize>,
}

thread_local! {
    static TASKS: RefCell<Tasks> = RefCell::default();
    static READY: ReadyQueue = Arc::default();
    /// A origem do efeito sendo criado agora: o `spawn_local` de dentro do `Effect::new` a leva
    /// para a tarefa.
    static ORIGIN: Cell<Option<EffectOrigin>> = const { Cell::new(None) };
    /// O tempo dos efeitos desta chamada de [`run_effects`], antes de ir para o total.
    static SPENT: RefCell<HashMap<EffectOrigin, (Duration, u32)>> = RefCell::default();
}

static PROFILING: AtomicBool = AtomicBool::new(false);
static TOTALS: Mutex<Option<HashMap<EffectOrigin, (Duration, u32)>>> = Mutex::new(None);

/// De onde um efeito veio: a linha que chamou [`effect`](crate::effect()), ou o nome dado em
/// [`effect_named`](crate::effect_named) (a árvore usa o tipo do widget, como `terminal`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EffectOrigin {
    /// A linha do código que criou o efeito.
    Code(&'static Location<'static>),
    /// O nome dado ao efeito.
    Named(&'static str),
}

impl fmt::Display for EffectOrigin {
    /// A linha sai sem o começo do caminho, a partir da pasta do crate (`core/src/tree.rs:367`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code(location) => {
                let file = location.file().replace('\\', "/");
                let short = file
                    .rsplit_once("crates/")
                    .map_or(file.as_str(), |(_, rest)| rest);
                write!(f, "{short}:{}", location.line())
            }
            Self::Named(name) => f.write_str(name),
        }
    }
}

/// O tempo que os efeitos de uma origem gastaram, e quantas vezes rodaram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectTotal {
    /// De onde os efeitos vieram.
    pub origin: EffectOrigin,
    /// O tempo somado dos `poll` deles.
    pub time: Duration,
    /// Quantas vezes rodaram.
    pub count: u32,
}

/// Liga (ou desliga) a medição do tempo de cada efeito, para o processo todo: o registro de
/// travamentos diz quais origens gastaram mais. Desligada (o padrão), custa o teste de um `bool`
/// por efeito.
pub fn profile_effects(on: bool) {
    PROFILING.store(on, Ordering::Release);
}

/// O tempo dos efeitos por origem desde a última chamada, do que mais gastou para o que menos,
/// e zera a conta. Vazio com a medição desligada.
pub fn take_effect_totals() -> Vec<EffectTotal> {
    let totals = TOTALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .unwrap_or_default();
    let mut list: Vec<EffectTotal> = totals
        .into_iter()
        .map(|(origin, (time, count))| EffectTotal {
            origin,
            time,
            count,
        })
        .collect();
    list.sort_by(|a, b| {
        b.time
            .cmp(&a.time)
            .then_with(|| a.origin.to_string().cmp(&b.origin.to_string()))
    });
    list
}

/// Cria o efeito com `create` marcando a tarefa dele com `origin`.
pub(crate) fn with_origin<T>(origin: EffectOrigin, create: impl FnOnce() -> T) -> T {
    let before = ORIGIN.replace(Some(origin));
    let made = create();
    ORIGIN.set(before);
    made
}

/// Passa o tempo desta chamada de [`run_effects`] para o total que a vigia lê.
fn flush_spent() {
    let spent = SPENT.with_borrow_mut(std::mem::take);
    if spent.is_empty() {
        return;
    }
    let mut totals = TOTALS.lock().unwrap_or_else(PoisonError::into_inner);
    let totals = totals.get_or_insert_with(HashMap::new);
    for (origin, (time, count)) in spent {
        let total = totals.entry(origin).or_default();
        total.0 += time;
        total.1 += count;
    }
}

struct TaskWaker {
    id: TaskId,
    ready: ReadyQueue,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let mut ready = self.ready.lock().unwrap_or_else(|e| e.into_inner());
        if !ready.contains(&self.id) {
            ready.push_back(self.id);
        }
    }
}

struct FrameExecutor;

impl CustomExecutor for FrameExecutor {
    fn spawn(&self, fut: PinnedFuture<()>) {
        // Tarefas `Send` também rodam na thread da UI, no próximo frame.
        self.spawn_local(fut);
    }

    fn spawn_local(&self, fut: PinnedLocalFuture<()>) {
        let id = TASKS.with_borrow_mut(|tasks| {
            let index = tasks.free.pop().unwrap_or_else(|| {
                tasks.slots.push(Slot::default());
                tasks.slots.len() - 1
            });
            let slot = &mut tasks.slots[index];
            slot.generation += 1;
            slot.task = Some(fut);
            slot.origin = ORIGIN.take();
            TaskId {
                index,
                generation: slot.generation,
            }
        });
        READY.with(|ready| {
            ready
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(id)
        });
    }

    fn poll_local(&self) {
        run_effects();
    }
}

/// Instala o executor de frame na thread atual. Pode ser chamada várias vezes.
///
/// [`Scope::root`] já chama esta função.
///
/// # Panics
///
/// Se outro executor do `any_spawner` (tokio, wasm-bindgen…) já tiver sido instalado no
/// processo: os efeitos iriam para ele e nunca rodariam no frame.
pub fn init() {
    // O `any_spawner` guarda as funções num `OnceLock` global, que consultam uma instância por
    // thread: cada thread registra a sua, e o `AlreadySet` das seguintes é esperado.
    let _ = Executor::init_local_custom_executor(FrameExecutor);
    let before = live_tasks();
    Executor::spawn_local(async {});
    assert!(
        live_tasks() > before,
        "outro executor do any_spawner já está instalado; o Tomni UI precisa do executor de frame"
    );
    run_effects();
}

/// Roda os efeitos pendentes, incluindo os que ficarem pendentes durante a chamada, até não
/// sobrar nenhum. Devolve quantas tarefas foram executadas.
pub fn run_effects() -> usize {
    let ready = READY.with(Arc::clone);
    let profiling = PROFILING.load(Ordering::Acquire);
    let mut polls = 0;
    loop {
        let next = ready.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
        let Some(id) = next else {
            if profiling {
                flush_spent();
            }
            return polls;
        };
        // Tira a tarefa da tabela para que ela possa criar outras durante o `poll`.
        let task = TASKS.with_borrow_mut(|tasks| {
            let slot = &mut tasks.slots[id.index];
            (slot.generation == id.generation)
                .then(|| slot.task.take().map(|task| (task, slot.origin)))
                .flatten()
        });
        let Some((mut task, origin)) = task else {
            continue;
        };
        let started = profiling.then(Instant::now);
        let waker = Waker::from(Arc::new(TaskWaker {
            id,
            ready: Arc::clone(&ready),
        }));
        polls += 1;
        let pending = task
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending();
        if let (Some(started), Some(origin)) = (started, origin) {
            let took = started.elapsed();
            SPENT.with_borrow_mut(|spent| {
                let total = spent.entry(origin).or_default();
                total.0 += took;
                total.1 += 1;
            });
        }
        if pending {
            TASKS.with_borrow_mut(|tasks| tasks.slots[id.index].task = Some(task));
        } else {
            // Fora do empréstimo: soltar a tarefa pode rodar `on_cleanup`, que pode criar tarefas.
            drop(task);
            TASKS.with_borrow_mut(|tasks| {
                tasks.slots[id.index].origin = None;
                tasks.free.push(id.index);
            });
        }
    }
}

/// Tarefas ainda vivas nesta thread. Serve para os testes detectarem efeitos que vazaram.
pub fn live_tasks() -> usize {
    TASKS.with_borrow(|tasks| tasks.slots.len() - tasks.free.len())
}

/// Dono dos signals, memos e efeitos criados dentro de [`Scope::run`].
///
/// Cada widget tem o seu, filho do escopo do pai. Quando o widget sai da árvore,
/// [`Scope::clear`] (ou soltar o último handle) descarta tudo o que foi criado nele e nos
/// filhos.
#[derive(Clone, Debug)]
pub struct Scope(Owner);

impl Scope {
    /// Escopo raiz da árvore. Instala o executor de frame na thread atual.
    ///
    /// Se for chamada dentro de [`Scope::run`], o escopo novo vira filho do atual.
    pub fn root() -> Self {
        init();
        Self(Owner::new())
    }

    /// Escopo raiz que não é filho do escopo atual, mesmo chamado dentro de [`Scope::run`]:
    /// vive até ser descartado de propósito, e não junto do widget que o criou. Serve a quem
    /// guarda signals fora da árvore (as sessões do terminal).
    pub fn detached() -> Self {
        init();
        let current = Owner::current();
        if let Some(current) = &current {
            current.clone().unset();
        }
        let owner = Owner::new();
        if let Some(current) = current {
            current.set();
        }
        Self(owner)
    }

    /// Escopo filho: é descartado junto com este.
    pub fn child(&self) -> Self {
        Self(self.0.child())
    }

    /// Roda `f` com este escopo como dono de tudo o que for criado.
    pub fn run<T>(&self, f: impl FnOnce() -> T) -> T {
        self.0.with(f)
    }

    /// Descarta os filhos, roda as funções de [`on_cleanup`] e libera os signals, memos e
    /// efeitos criados neste escopo. O escopo continua utilizável, por exemplo para montar o
    /// widget de novo.
    pub fn clear(&self) {
        self.0.cleanup();
    }
}

/// Registra `f` para rodar quando o escopo atual for descartado.
pub fn on_cleanup(f: impl FnOnce() + Send + Sync + 'static) {
    Owner::on_cleanup(f);
}
