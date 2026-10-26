//! As chamadas Win32 que nenhum crate expõe com API segura (M44). Único crate com `unsafe`.
//!
//! O Pageant (o agente do PuTTY) e o pid de quem pede uma chave precisam de
//! `CryptProtectMemory`, `GetUserNameExA`, uma janela oculta com `WM_COPYDATA`, um file mapping
//! com a checagem do dono, `GetNamedPipeClientProcessId`, `CreateToolhelp32Snapshot` e um pipe
//! com security descriptor própria. A pesquisa não achou crate com API segura para nenhuma
//! delas, e o usuário aprovou o `unsafe` contido aqui em 19/10.
//!
//! Cada bloco `unsafe` tem um `// SAFETY:` que diz por que a chamada é válida, tem uma única
//! operação insegura, e nenhuma função pública do crate é `unsafe`. O que não precisa de
//! `unsafe` (o hash e o nome do pipe, ler e escrever a mensagem no mapa) fica em
//! `tomni_ui_terminal::ssh::agent`.
#![cfg(windows)]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

mod crypto;
mod pipe;
mod process;
mod window;

#[cfg(test)]
mod tests;

pub use crypto::{pageant_user_name, protect_memory_cross_process};
pub use pipe::{named_pipe_client_pid, private_pipe};
pub use process::{ProcessEntry, process_table};
pub use window::{CopyDataWindow, window_exists};

use std::io;
use windows::Win32::Foundation::{CloseHandle, HANDLE};

/// Um `HANDLE` do Win32 que se fecha ao ser solto.
pub(crate) struct Handle(pub(crate) HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: o `Handle` é o dono único do `HANDLE`, que veio de uma chamada que o abriu e
        // ainda não foi fechado; o resultado é ignorado porque não há o que fazer num `drop`.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// O erro do `windows` como `io::Error` (o código do sistema vira o `kind`).
pub(crate) fn io_error(erro: windows::core::Error) -> io::Error {
    io::Error::from(erro)
}
