//! O pipe do servidor com dono e acesso só do usuário, e o pid de quem se conecta nele.
//!
//! O cliente do PuTTY (`windows/named-pipe-client.c`) confere o dono do pipe: tem de ser o SID
//! do usuário. Um pipe criado com a segurança padrão por um processo elevado fica com o dono
//! "Administradores", e o PuTTY recusa. Por isso o pipe nasce com uma security descriptor própria
//! (dono = o usuário; acesso só para ele e para o SYSTEM), como o `make_private_security_descriptor`
//! do PuTTY. O `tokio` só aceita isso por `create_with_security_attributes_raw`, que é `unsafe`.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::AsRawHandle;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{HSTRING, PCWSTR, PWSTR};

use crate::{Handle, io_error};

/// O SID do usuário deste processo, como texto (`S-1-5-21-…`).
fn user_sid_string() -> io::Result<String> {
    let mut token = HANDLE::default();
    // SAFETY: devolve o pseudo-handle do próprio processo, que não precisa ser fechado.
    let processo = unsafe { GetCurrentProcess() };
    // SAFETY: `processo` é um handle válido e `token` é um `HANDLE` válido onde a chamada grava
    // o handle novo.
    unsafe { OpenProcessToken(processo, TOKEN_QUERY, &mut token) }.map_err(io_error)?;
    let token = Handle(token);

    let mut tamanho = 0u32;
    // SAFETY: sem buffer, só devolve em `tamanho` quanto é preciso (e falha com
    // `ERROR_INSUFFICIENT_BUFFER`, o esperado).
    let _ = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut tamanho) };
    // Um `Vec<u64>` garante o alinhamento de 8 bytes que o `TOKEN_USER` pede.
    let mut buffer = vec![0u64; (tamanho as usize).div_ceil(8).max(1)];
    // SAFETY: `buffer` tem pelo menos `tamanho` bytes, que é o que se diz à chamada.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            tamanho,
            &mut tamanho,
        )
    }
    .map_err(io_error)?;
    // SAFETY: a chamada acima preencheu o início do `buffer` (alinhado a 8 bytes) com um
    // `TOKEN_USER`, que fica vivo enquanto `buffer` viver.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };

    let mut texto = PWSTR::null();
    // SAFETY: `sid` aponta para um SID válido dentro do `buffer`; a chamada aloca o texto em
    // `texto`, que é liberado com `LocalFree` logo abaixo.
    unsafe { ConvertSidToStringSidW(sid, &mut texto) }.map_err(io_error)?;
    // SAFETY: `texto` é um texto terminado em NUL que a chamada acima acabou de alocar.
    let sid_texto = unsafe { texto.to_string() };
    // SAFETY: `texto` foi alocado pela chamada acima com `LocalAlloc`, e não é usado depois.
    let _ = unsafe { LocalFree(Some(HLOCAL(texto.0.cast()))) };
    sid_texto.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Um pipe do servidor com dono = o usuário e acesso só do usuário e do SYSTEM.
///
/// `first_instance` pede que o nome ainda não exista (`FILE_FLAG_FIRST_PIPE_INSTANCE`): quem
/// já tem o pipe faz o erro ser `PermissionDenied`. Só vale dentro de um runtime do `tokio`.
pub fn private_pipe(name: &str, first_instance: bool) -> io::Result<NamedPipeServer> {
    let sid = user_sid_string()?;
    // Dono = o usuário; a DACL é protegida e libera tudo (`GA`) só para ele e para o SYSTEM.
    let sddl = HSTRING::from(format!("O:{sid}D:P(A;;GA;;;{sid})(A;;GA;;;SY)"));
    let mut descritor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `sddl` é um texto terminado em NUL que vive durante a chamada, e `descritor` é um
    // ponteiro válido onde ela grava a descriptor, que é liberada com `LocalFree` abaixo.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descritor,
            None,
        )
    }
    .map_err(io_error)?;
    let mut atributos = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descritor.0,
        bInheritHandle: false.into(),
    };
    let pipe = {
        let mut opcoes = ServerOptions::new();
        opcoes.first_pipe_instance(first_instance);
        let ponteiro = (&mut atributos as *mut SECURITY_ATTRIBUTES).cast::<c_void>();
        // SAFETY: `atributos` é um `SECURITY_ATTRIBUTES` válido, com a descriptor viva, e
        // continua vivo durante a chamada, que não guarda o ponteiro depois de voltar.
        unsafe { opcoes.create_with_security_attributes_raw(name, ponteiro) }
    };
    // SAFETY: `descritor.0` foi alocado pela chamada acima com `LocalAlloc`, e o `tokio` já
    // copiou o que precisava dele ao criar o pipe.
    let _ = unsafe { LocalFree(Some(HLOCAL(descritor.0))) };
    pipe
}

/// `GetNamedPipeClientProcessId`: o pid de quem está do outro lado de `pipe`.
pub fn named_pipe_client_pid(pipe: &NamedPipeServer) -> io::Result<u32> {
    let mut pid = 0u32;
    let handle = HANDLE(pipe.as_raw_handle());
    // SAFETY: `handle` é o handle de um pipe do servidor que `pipe` mantém aberto durante a
    // chamada, e `pid` é um `u32` válido onde ela grava o resultado.
    unsafe { GetNamedPipeClientProcessId(handle, &mut pid) }.map_err(io_error)?;
    Ok(pid)
}
