//! [`Live`]: um valor que uma thread de trabalho grava e a tela lê como um signal.
//!
//! Os signals do runtime vivem na thread da interface. Aqui o valor mora num `Arc<Mutex<T>>`
//! e o signal é só um contador de versão, que a thread sobe sob um mutex estático (o mesmo que
//! protege a leitura) antes de acordar a interface. A tela lê com [`Live::get`], que rastreia
//! o contador, e o efeito roda no próximo frame.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tomni_ui_reactive::{Get, GetUntracked, Scope, Signal, Update, signal};

thread_local! {
    /// O escopo dos valores de [`Live::detached`]: vive até a thread acabar.
    static DETACHED: Scope = Scope::detached();
}

/// Serializa o acesso aos contadores entre as threads de trabalho e a da interface.
static ACCESS: Mutex<()> = Mutex::new(());

fn lock_access() -> MutexGuard<'static, ()> {
    ACCESS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Um valor gravado de qualquer thread e lido, com rastreamento, na da interface. Crie na
/// thread da interface (o contador nasce no escopo atual).
pub struct Live<T> {
    data: Arc<Mutex<T>>,
    version: Signal<u64>,
}

impl<T> Clone for Live<T> {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
            version: self.version,
        }
    }
}

impl<T> std::fmt::Debug for Live<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live").finish_non_exhaustive()
    }
}

impl<T> Live<T> {
    /// Cria o valor com `value`.
    pub fn new(value: T) -> Self {
        Self {
            data: Arc::new(Mutex::new(value)),
            version: signal(0),
        }
    }

    /// Como [`Live::new`], mas o contador não morre com o escopo que está ativo: serve às
    /// respostas pedidas de dentro de um evento (arrastar, soltar), cujo escopo acaba antes de
    /// a thread de trabalho responder. Cada valor fica até a thread da interface acabar.
    pub fn detached(value: T) -> Self {
        Self {
            data: Arc::new(Mutex::new(value)),
            version: DETACHED.with(|scope| scope.run(|| signal(0))),
        }
    }

    fn data(&self) -> MutexGuard<'_, T> {
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lê o valor e rastreia a mudança (para views e efeitos).
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        {
            let _access = lock_access();
            let _ = self.version.get();
        }
        f(&self.data())
    }

    /// Lê o valor sem rastrear.
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        {
            let _access = lock_access();
            let _ = self.version.get_untracked();
        }
        f(&self.data())
    }

    /// Muda o valor, de qualquer thread, e acorda a interface.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.data());
        self.bump();
    }

    /// Troca o valor, de qualquer thread, e acorda a interface.
    pub fn set(&self, value: T) {
        *self.data() = value;
        self.bump();
    }

    fn bump(&self) {
        {
            let _access = lock_access();
            let _ = self.version.try_update(|v| *v += 1);
        }
        tomni_ui_core::wake();
    }
}

impl<T: Clone> Live<T> {
    /// Uma cópia do valor, com rastreamento.
    pub fn get(&self) -> T {
        self.with(T::clone)
    }

    /// Uma cópia do valor, sem rastreamento.
    pub fn get_untracked(&self) -> T {
        self.with_untracked(T::clone)
    }
}
