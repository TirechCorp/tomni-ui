//! A saúde dos discos pelo `IOCTL_STORAGE_QUERY_PROPERTY`, sem o `smartctl`: os bytes que o
//! disco devolve, que quem chama interpreta.
//!
//! Para cada `\\.\PhysicalDriveN`, o `StorageDeviceProperty` dá o modelo, o fornecedor e o
//! barramento, e o `StorageAccessAlignmentProperty`, o tamanho do setor lógico. Depois, pelo
//! `StorageDeviceProtocolSpecificProperty`:
//!
//! - num NVMe, o registro de saúde (a página de log 2, com 512 bytes);
//! - num SATA, as páginas 1 (geral), 3 (disco rotativo) e 7 (SSD) das estatísticas do aparelho
//!   (o log 0x04 do ATA), com 512 bytes cada.
//!
//! O disco é aberto primeiro sem direito nenhum, o que basta em muitos discos. Se o Windows
//! recusar, a leitura tenta de novo abrindo com leitura e escrita, o que só funciona para o
//! administrador; recusado de novo, o disco volta com [`DiskHealth::access_denied`]. Nada pede
//! elevação.

use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    AtaDataTypeLogPage, IOCTL_STORAGE_QUERY_PROPERTY, NVMeDataTypeLogPage, PropertyStandardQuery,
    ProtocolTypeAta, ProtocolTypeNvme, StorageAccessAlignmentProperty, StorageDeviceProperty,
    StorageDeviceProtocolSpecificProperty,
};
use windows::core::{HRESULT, HSTRING};

use crate::Handle;
use crate::sensors::{em_bytes, propriedade};

/// Até que número de `\\.\PhysicalDriveN` se procura (o mesmo do `disk_counters`).
const DISCOS: u32 = 32;
/// O tamanho de um registro: o de saúde do NVMe e cada página das estatísticas do ATA.
const REGISTRO: usize = 512;
/// O cabeçalho do `STORAGE_PROPERTY_QUERY` e do `STORAGE_PROTOCOL_DATA_DESCRIPTOR` (o
/// `PropertyId` e o `QueryType`, ou o `Version` e o `Size`).
const CABECALHO: usize = 8;
/// O tamanho do `STORAGE_PROTOCOL_SPECIFIC_DATA`, logo depois do cabeçalho.
const PROTOCOLO: usize = 40;
/// O espaço para os dados, o mesmo do exemplo da Microsoft (`NVME_MAX_LOG_SIZE`).
const ESPACO: usize = 4096;
/// O registro de saúde do NVMe (`NVME_LOG_PAGE_HEALTH_INFO`).
const LOG_SAUDE_NVME: u32 = 2;
/// As estatísticas do aparelho no ATA (`IDE_GP_LOG_DEVICE_STATISTICS`).
const LOG_ESTATISTICAS_ATA: u32 = 4;
/// As páginas das estatísticas que se leem: a geral, a do disco rotativo e a do SSD.
const PAGINAS_ATA: [u32; 3] = [1, 3, 7];

/// O barramento de um disco ([`DiskHealth::bus`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskBus {
    /// NVMe.
    Nvme,
    /// SATA ou ATA.
    Ata,
    /// USB (o adaptador quase nunca passa os comandos adiante).
    Usb,
    /// Um disco virtual (`BusTypeVirtual`, `BusTypeFileBackedVirtual`).
    Virtual,
    /// Outro (SCSI, SAS, RAID, Espaços de Armazenamento…).
    Other,
}

/// O que o disco `\\.\PhysicalDriveN` devolveu ([`disk_health`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskHealth {
    /// O `N` de `\\.\PhysicalDriveN`, o mesmo do [`crate::DiskCounters`].
    pub index: u32,
    /// O modelo (o `ProductId`).
    pub model: Option<String>,
    /// O fornecedor (o `VendorId`), que nos discos virtuais costuma dizer de onde vêm.
    pub vendor: Option<String>,
    /// O barramento.
    pub bus: DiskBus,
    /// Num NVMe, o registro de saúde (a página de log 2), com 512 bytes.
    pub nvme_smart_log: Option<Vec<u8>>,
    /// Num SATA, as páginas das estatísticas do aparelho que o disco devolveu (512 bytes cada,
    /// com o número da página no byte 2).
    pub ata_device_statistics: Vec<Vec<u8>>,
    /// O tamanho do setor lógico, em bytes (512 quando o disco não diz).
    pub logical_sector_size: u32,
    /// O Windows recusou a leitura (é preciso ser administrador).
    pub access_denied: bool,
}

/// Os discos físicos deste computador, com o que cada um devolveu.
pub fn disk_health() -> Vec<DiskHealth> {
    (0..DISCOS).filter_map(disco).collect()
}

/// Por que uma leitura falhou.
enum Falha {
    /// O Windows recusou (`ERROR_ACCESS_DENIED`).
    Negada,
    /// Outra coisa: o disco não tem o registro, ou o driver não passa o pedido.
    Outra,
}

impl From<windows::core::Error> for Falha {
    fn from(erro: windows::core::Error) -> Self {
        if erro.code() == HRESULT::from_win32(ERROR_ACCESS_DENIED.0) {
            Falha::Negada
        } else {
            Falha::Outra
        }
    }
}

/// Abre o disco `n` com o acesso pedido.
fn abrir(n: u32, acesso: u32) -> Result<Handle, Falha> {
    let caminho = HSTRING::from(format!(r"\\.\PhysicalDrive{n}"));
    // SAFETY: o caminho é uma string terminada em zero que vive até o fim da chamada, e o
    // `Handle` fecha o que abrir.
    let aberto = unsafe {
        CreateFileW(
            &caminho,
            acesso,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None::<HANDLE>,
        )
    }?;
    Ok(Handle(aberto))
}

/// O que se sabe do disco `n`, se ele existe.
fn disco(n: u32) -> Option<DiskHealth> {
    let aberto = abrir(n, 0).ok()?;
    let (model, vendor, bus) = descritor(aberto.0)?;
    let mut saude = DiskHealth {
        index: n,
        model,
        vendor,
        bus,
        nvme_smart_log: None,
        ata_device_statistics: Vec::new(),
        logical_sector_size: setor_logico(aberto.0).unwrap_or(512),
        access_denied: false,
    };
    if !matches!(bus, DiskBus::Nvme | DiskBus::Ata) {
        return Some(saude);
    }
    let lido = match registros(aberto.0, bus) {
        Err(Falha::Negada) => {
            drop(aberto);
            abrir(n, GENERIC_READ.0 | GENERIC_WRITE.0)
                .and_then(|com_direito| registros(com_direito.0, bus))
        }
        outro => outro,
    };
    match lido {
        Ok((nvme, ata)) => {
            saude.nvme_smart_log = nvme;
            saude.ata_device_statistics = ata;
        }
        Err(Falha::Negada) => saude.access_denied = true,
        Err(Falha::Outra) => {}
    }
    Some(saude)
}

/// O que um disco devolveu: o registro de saúde do NVMe ou as páginas do ATA.
type Registros = (Option<Vec<u8>>, Vec<Vec<u8>>);

/// O registro de saúde do NVMe, ou as páginas das estatísticas do ATA.
fn registros(disco: HANDLE, bus: DiskBus) -> Result<Registros, Falha> {
    if bus == DiskBus::Nvme {
        let registro = protocolo(
            disco,
            ProtocolTypeNvme.0,
            NVMeDataTypeLogPage.0,
            LOG_SAUDE_NVME,
            0,
        )?;
        return Ok((Some(registro), Vec::new()));
    }
    let mut paginas = Vec::new();
    for (i, pagina) in PAGINAS_ATA.into_iter().enumerate() {
        let lida = protocolo(
            disco,
            ProtocolTypeAta.0,
            AtaDataTypeLogPage.0,
            LOG_ESTATISTICAS_ATA,
            pagina,
        );
        match lida {
            Ok(lida) => paginas.push(lida),
            // A geral decide: recusada, o disco todo foi recusado. As outras podem faltar (a do
            // SSD num disco rotativo, e vice-versa).
            Err(falha) if i == 0 => return Err(falha),
            Err(_) => {}
        }
    }
    Ok((None, paginas))
}

/// Um pedido do `StorageDeviceProtocolSpecificProperty`: o `STORAGE_PROPERTY_QUERY` com o
/// `STORAGE_PROTOCOL_SPECIFIC_DATA` no lugar dos `AdditionalParameters`, e a resposta no mesmo
/// buffer (o `STORAGE_PROTOCOL_DATA_DESCRIPTOR`, com os dados no `ProtocolDataOffset`).
fn protocolo(
    disco: HANDLE,
    tipo: i32,
    dado: i32,
    valor: u32,
    subvalor: u32,
) -> Result<Vec<u8>, Falha> {
    let mut buffer = vec![0u32; (CABECALHO + PROTOCOLO + ESPACO) / 4];
    buffer[0] = StorageDeviceProtocolSpecificProperty.0 as u32;
    buffer[1] = PropertyStandardQuery.0 as u32;
    // `ProtocolType`, `DataType`, `ProtocolDataRequestValue`, `ProtocolDataRequestSubValue`,
    // `ProtocolDataOffset` e `ProtocolDataLength`; o resto fica em zero.
    buffer[2..8].copy_from_slice(&[
        tipo as u32,
        dado as u32,
        valor,
        subvalor,
        PROTOCOLO as u32,
        REGISTRO as u32,
    ]);
    let tamanho = size_of_val(buffer.as_slice()) as u32;
    let ponteiro = buffer.as_mut_ptr();
    let mut escritos = 0u32;
    // SAFETY: a entrada e a saída são o mesmo buffer local, do tamanho passado (como no exemplo
    // da Microsoft); `escritos` é local; a chamada é síncrona (sem `OVERLAPPED`).
    unsafe {
        DeviceIoControl(
            disco,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(ponteiro.cast_const().cast()),
            tamanho,
            Some(ponteiro.cast()),
            tamanho,
            Some(&mut escritos),
            None,
        )
    }?;
    let bytes: Vec<u8> = buffer.iter().flat_map(|p| p.to_le_bytes()).collect();
    let escritos = (escritos as usize).min(bytes.len());
    // O `ProtocolDataOffset` e o `ProtocolDataLength` da resposta, contados do começo do
    // `STORAGE_PROTOCOL_SPECIFIC_DATA`.
    let inicio = CABECALHO + palavra(&bytes, CABECALHO + 16)? as usize;
    let comprimento = palavra(&bytes, CABECALHO + 20)? as usize;
    if comprimento < REGISTRO || inicio < CABECALHO + PROTOCOLO || inicio + REGISTRO > escritos {
        return Err(Falha::Outra);
    }
    Ok(bytes[inicio..inicio + REGISTRO].to_vec())
}

/// O `u32` em `inicio`.
fn palavra(bytes: &[u8], inicio: usize) -> Result<u32, Falha> {
    bytes
        .get(inicio..inicio + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(Falha::Outra)
}

/// O modelo, o fornecedor e o barramento, do `STORAGE_DEVICE_DESCRIPTOR`.
fn descritor(disco: HANDLE) -> Option<(Option<String>, Option<String>, DiskBus)> {
    let dados = propriedade(disco, StorageDeviceProperty, 1024)?;
    let bytes = em_bytes(&dados);
    let numero = |inicio: usize| palavra(bytes, inicio).ok();
    // `VendorIdOffset`, `ProductIdOffset` e `BusType`.
    let (fornecedor, modelo, barramento) = (numero(12)?, numero(16)?, numero(28)?);
    let bus = match barramento {
        17 => DiskBus::Nvme,
        3 | 11 => DiskBus::Ata,
        7 => DiskBus::Usb,
        14 | 15 => DiskBus::Virtual,
        _ => DiskBus::Other,
    };
    Some((texto(bytes, modelo), texto(bytes, fornecedor), bus))
}

/// O texto terminado em zero em `inicio` (zero: não há).
fn texto(bytes: &[u8], inicio: u32) -> Option<String> {
    let inicio = usize::try_from(inicio).ok()?;
    if inicio == 0 || inicio >= bytes.len() {
        return None;
    }
    let resto = &bytes[inicio..];
    let fim = resto.iter().position(|&b| b == 0).unwrap_or(resto.len());
    let texto = String::from_utf8_lossy(&resto[..fim]).trim().to_owned();
    (!texto.is_empty()).then_some(texto)
}

/// O `BytesPerLogicalSector` do `STORAGE_ACCESS_ALIGNMENT_DESCRIPTOR`.
fn setor_logico(disco: HANDLE) -> Option<u32> {
    let dados = propriedade(disco, StorageAccessAlignmentProperty, 64)?;
    palavra(em_bytes(&dados), 16).ok().filter(|&s| s > 0)
}
