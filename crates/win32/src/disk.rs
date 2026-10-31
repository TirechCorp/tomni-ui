//! As operações e os bytes de cada disco físico, para a aba Disco do app.

use std::io;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{DISK_PERFORMANCE, IOCTL_DISK_PERFORMANCE};
use windows::core::HSTRING;

use crate::{Handle, io_error};

/// Até que número de `\\.\PhysicalDriveN` se procura.
const DISCOS: u32 = 32;

/// Os contadores de um disco físico ([`disk_counters`]), desde o boot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiskCounters {
    /// O número do disco, o do `\\.\PhysicalDriveN` (o "Disco 0" do Gerenciador de Tarefas).
    pub index: u32,
    /// As leituras feitas.
    pub read_count: u64,
    /// As escritas feitas.
    pub write_count: u64,
    /// Os bytes lidos.
    pub read_bytes: u64,
    /// Os bytes escritos.
    pub written_bytes: u64,
}

/// `IOCTL_DISK_PERFORMANCE` em cada `\\.\PhysicalDrive0` a `31`: os discos que abrem (sem
/// pedir acesso de leitura, então sem ser administrador) e respondem. Os outros números ficam
/// de fora. O Windows conta as operações em 32 bits: depois de 4 bilhões, a contagem volta a
/// zero.
pub fn disk_counters() -> io::Result<Vec<DiskCounters>> {
    Ok((0..DISCOS).filter_map(|n| contadores(n).ok()).collect())
}

/// Os contadores do disco `n`.
fn contadores(n: u32) -> io::Result<DiskCounters> {
    let caminho = HSTRING::from(format!(r"\\.\PhysicalDrive{n}"));
    // SAFETY: o caminho é uma string terminada em zero que vive até o fim da chamada; o acesso
    // zero só deixa consultar, e o `Handle` fecha o que abrir.
    let disco = unsafe {
        CreateFileW(
            &caminho,
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None::<HANDLE>,
        )
    }
    .map_err(io_error)?;
    let disco = Handle(disco);
    let mut desempenho = DISK_PERFORMANCE::default();
    let mut escritos = 0u32;
    // SAFETY: a saída é uma `DISK_PERFORMANCE` local do tamanho passado, e `escritos` é local;
    // a chamada é síncrona (sem `OVERLAPPED`).
    unsafe {
        DeviceIoControl(
            disco.0,
            IOCTL_DISK_PERFORMANCE,
            None,
            0,
            Some((&raw mut desempenho).cast()),
            size_of::<DISK_PERFORMANCE>() as u32,
            Some(&mut escritos),
            None,
        )
    }
    .map_err(io_error)?;
    Ok(DiskCounters {
        index: n,
        read_count: u64::from(desempenho.ReadCount),
        write_count: u64::from(desempenho.WriteCount),
        read_bytes: u64::try_from(desempenho.BytesRead).unwrap_or(0),
        written_bytes: u64::try_from(desempenho.BytesWritten).unwrap_or(0),
    })
}
