//! Runtime reativo: `signal`, `memo` e `effect`, no estilo `ref`/`computed`/`watchEffect` do Vue.
//!
//! A função de view roda uma vez; cada mudança marca como dirty só os widgets dependentes.
//! Signals vivem na thread da UI e as mudanças são agrupadas por frame: os efeitos só rodam em
//! [`run_effects`], chamado pelo loop de frame ou pelo `TestApp`.
//!
//! Por baixo é o `reactive_graph` (MIT), adotado no M1 depois de um
//! protótipo. Apps e widgets usam só esta fachada, para que o runtime possa ser
//! trocado sem mudar a API.
//!
//! ```
//! use tomni_ui_reactive::prelude::*;
//!
//! let scope = Scope::root();
//! scope.run(|| {
//!     let email = signal(String::new());
//!     let valido = memo(move || email.get().contains('@'));
//!     effect(move || println!("válido: {}", valido.get()));
//!
//!     email.set("ana@x.com".into());
//!     run_effects();
//! });
//! scope.clear();
//! ```

mod runtime;

use reactive_graph::computed;
use reactive_graph::effect;
use reactive_graph::owner::LocalStorage;

pub use reactive_graph::traits::{
    Get, GetUntracked, Set, Track, Update, UpdateUntracked, With, WithUntracked,
};
pub use runtime::{Scope, init, live_tasks, on_cleanup, run_effects};

/// Roda `f` sem registrar dependências: o que for lido dentro não faz o efeito ou memo atual
/// rodar de novo.
pub fn untrack<T>(f: impl FnOnce() -> T) -> T {
    reactive_graph::graph::untrack(f)
}

/// Valor reativo. O handle é `Copy` e vale enquanto o [`Scope`] que o criou existir.
pub type Signal<T> = reactive_graph::signal::RwSignal<T>;

/// Valor derivado com cache: só notifica os dependentes quando o resultado muda (`PartialEq`).
pub type Memo<T> = computed::Memo<T>;

/// Efeito colateral registrado com [`effect()`].
pub type Effect = effect::Effect<LocalStorage>;

/// Cria um valor reativo (`ref` do Vue).
#[track_caller]
pub fn signal<T: Send + Sync + 'static>(value: T) -> Signal<T> {
    Signal::new(value)
}

/// Cria um valor derivado de outros (`computed` do Vue). `f` roda de novo só quando um valor
/// lido nela muda, e os dependentes só são notificados se o resultado for diferente.
#[track_caller]
pub fn memo<T>(f: impl Fn() -> T + Send + Sync + 'static) -> Memo<T>
where
    T: PartialEq + Send + Sync + 'static,
{
    Memo::new(move |_| f())
}

/// Registra um efeito colateral (`watchEffect` do Vue). Roda no próximo [`run_effects`] e de
/// novo sempre que um valor lido nele mudar; é descartado junto com o [`Scope`] atual.
#[track_caller]
pub fn effect(f: impl FnMut() + 'static) -> Effect {
    Effect::new(f)
}

/// Tudo o que uma view precisa: `use tomni_ui_reactive::prelude::*`.
pub mod prelude {
    pub use crate::{
        Get, GetUntracked, Memo, Scope, Set, Signal, Update, With, WithUntracked, effect, memo,
        on_cleanup, run_effects, signal, untrack,
    };
}

#[cfg(test)]
mod tests;
