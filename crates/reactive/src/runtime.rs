//! Executor de frame e escopos de dono.
//!
//! O `reactive_graph` agenda cada efeito como uma tarefa via `any_spawner`. Aqui essas tarefas
//! vão para uma fila por thread que só anda quando o loop de frame (ou o `TestApp`) chama
//! [`run_effects`]: nada roda dentro de um `set`, e várias mudanças no mesmo frame viram uma
//! execução só.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Wake, Waker};

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
    let mut polls = 0;
    loop {
        let next = ready.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
        let Some(id) = next else {
            return polls;
        };
        // Tira a tarefa da tabela para que ela possa criar outras durante o `poll`.
        let task = TASKS.with_borrow_mut(|tasks| {
            let slot = &mut tasks.slots[id.index];
            (slot.generation == id.generation)
                .then(|| slot.task.take())
                .flatten()
        });
        let Some(mut task) = task else {
            continue;
        };
        let waker = Waker::from(Arc::new(TaskWaker {
            id,
            ready: Arc::clone(&ready),
        }));
        polls += 1;
        let pending = task
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending();
        if pending {
            TASKS.with_borrow_mut(|tasks| tasks.slots[id.index].task = Some(task));
        } else {
            // Fora do empréstimo: soltar a tarefa pode rodar `on_cleanup`, que pode criar tarefas.
            drop(task);
            TASKS.with_borrow_mut(|tasks| tasks.free.push(id.index));
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
