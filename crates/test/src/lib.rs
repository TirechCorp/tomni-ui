//! `TestApp` headless e harness de snapshot sobre o backend CPU.
//!
//! Interage por `id` (`type_into`, `enabled`), conta repaints por widget e compara snapshots
//! com a referência (limite de 0,5% dos pixels). O harness é parte do marco M0; o `TestApp`,
//! do M2. Com `TOMNI_UI_BACKEND=gpu` e a feature `gpu`, os snapshots são rasterizados na GPU
//! e comparados com as referências da CPU (M4); veja [`backend`].

mod app;
pub mod backend;
pub mod snapshot;

pub use app::{DataGridInfo, GridColumnInfo, Keys, Spot, TestApp, TreeRowInfo};
pub use backend::{Backend, render};
pub use tomni_ui::{GridRowInfo, PropertyInfo};

/// Compara uma imagem com `snapshots/<nome>.png`, na pasta do crate que está sendo testado.
///
/// Sem referência ou com diferença acima de 0,5%, grava `<nome>.snap.new.png` (e o
/// `<nome>.snap.diff.png`) e falha. Para aprovar, rode com `TOMNI_UI_UPDATE_SNAPSHOTS=1`.
#[macro_export]
macro_rules! assert_snapshot {
    ($name:expr, $pixmap:expr $(,)?) => {
        $crate::snapshot::assert_snapshot(
            ::std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("snapshots"),
            $name,
            &$pixmap,
        )
    };
}
