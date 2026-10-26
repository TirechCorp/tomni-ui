//! `CryptProtectMemory` e o nome do usuário, como o PuTTY os usa no nome do pipe do Pageant.

use std::io;

use windows::Win32::Security::Authentication::Identity::{GetUserNameExA, NameUserPrincipal};
use windows::Win32::Security::Cryptography::{
    CRYPTPROTECTMEMORY_BLOCK_SIZE, CRYPTPROTECTMEMORY_CROSS_PROCESS, CryptProtectMemory,
};
use windows::Win32::System::WindowsProgramming::GetUserNameA;
use windows::core::PSTR;

use crate::io_error;

/// `CryptProtectMemory(…, CRYPTPROTECTMEMORY_CROSS_PROCESS)`: cifra `buf` no lugar, com uma
/// chave que vale para todos os processos da sessão do usuário. O tamanho tem de ser múltiplo
/// de 16 (e não vazio); senão, `InvalidInput`.
pub fn protect_memory_cross_process(buf: &mut [u8]) -> io::Result<()> {
    let bloco = CRYPTPROTECTMEMORY_BLOCK_SIZE as usize;
    if buf.is_empty() || !buf.len().is_multiple_of(bloco) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("o tamanho do buffer precisa ser múltiplo de {bloco}"),
        ));
    }
    let tamanho = u32::try_from(buf.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "o buffer é grande demais"))?;
    // SAFETY: `buf` é uma fatia mutável válida de `tamanho` bytes, que o `CryptProtectMemory`
    // lê e reescreve no lugar durante a chamada, e que é múltiplo do bloco de 16 bytes.
    unsafe {
        CryptProtectMemory(
            buf.as_mut_ptr().cast(),
            tamanho,
            CRYPTPROTECTMEMORY_CROSS_PROCESS,
        )
    }
    .map_err(io_error)
}

/// O nome do usuário como o PuTTY o usa: `GetUserNameExA(NameUserPrincipal)` cortado no `@`; se
/// falhar (máquina fora de domínio, o caso comum), `GetUserNameA`.
pub fn pageant_user_name() -> io::Result<String> {
    let mut tamanho = 0u32;
    // SAFETY: sem buffer, a chamada só devolve o tamanho necessário em `tamanho`, que é um
    // `u32` válido; o resultado (sempre `ERROR_MORE_DATA`) não importa.
    let _ = unsafe { GetUserNameExA(NameUserPrincipal, None, &mut tamanho) };
    let mut nome = vec![0u8; tamanho as usize];
    // SAFETY: `nome` tem `tamanho` bytes, que é o que se diz à chamada em `tamanho`.
    let pelo_dominio = tamanho > 0
        && unsafe {
            GetUserNameExA(
                NameUserPrincipal,
                Some(PSTR(nome.as_mut_ptr())),
                &mut tamanho,
            )
        };
    if !pelo_dominio {
        let mut tamanho = 0u32;
        // SAFETY: sem buffer, só devolve o tamanho necessário (falha com
        // `ERROR_INSUFFICIENT_BUFFER`, que é o esperado).
        let _ = unsafe { GetUserNameA(None, &mut tamanho) };
        nome = vec![0u8; tamanho as usize];
        // SAFETY: `nome` tem `tamanho` bytes, que é o que se diz à chamada em `tamanho`.
        unsafe { GetUserNameA(Some(PSTR(nome.as_mut_ptr())), &mut tamanho) }.map_err(io_error)?;
    }
    // Como o PuTTY: lê até o primeiro NUL e corta no primeiro `@`.
    let fim = nome.iter().position(|&b| b == 0).unwrap_or(nome.len());
    let nome = String::from_utf8_lossy(&nome[..fim]);
    Ok(nome.split('@').next().unwrap_or_default().to_owned())
}
