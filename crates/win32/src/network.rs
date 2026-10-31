//! O tráfego das interfaces de rede, para a aba Rede do app.

use std::io;

use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2, MIB_IF_TYPE_LOOPBACK,
};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

use crate::io_error;

/// O bit `HardwareInterface` dos `InterfaceAndOperStatusFlags`.
const HARDWARE: u8 = 1 << 0;
/// O bit `FilterInterface`: um filtro do NDIS em cima de outra interface, com o mesmo tráfego.
const FILTRO: u8 = 1 << 1;

/// Uma interface de rede ([`network_interfaces`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkInterface {
    /// O nome que o usuário vê ("Ethernet", "Wi-Fi").
    pub name: String,
    /// Os bytes recebidos desde que a interface subiu.
    pub received_bytes: u64,
    /// Os bytes enviados.
    pub sent_bytes: u64,
    /// Conectada (`IfOperStatusUp`).
    pub up: bool,
    /// Uma placa de verdade (`HardwareInterface`), e não uma VPN, uma ponte ou um túnel, que
    /// repetem o tráfego dela.
    pub physical: bool,
}

/// `GetIfTable2`: as interfaces de rede, sem a de loopback e sem os filtros do NDIS.
pub fn network_interfaces() -> io::Result<Vec<NetworkInterface>> {
    let mut tabela: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: `tabela` é local; a chamada grava nele uma tabela que ela mesma aloca.
    unsafe { GetIfTable2(&mut tabela) }.ok().map_err(io_error)?;
    let tabela = Tabela(tabela);
    // SAFETY: a chamada deu certo, então o ponteiro aponta para uma tabela válida até o
    // `FreeMibTable` do `drop`.
    let quantas = unsafe { (*tabela.0).NumEntries } as usize;
    // SAFETY: o mesmo ponteiro válido; só se toma o endereço da primeira linha, sem ler.
    let primeira: *const MIB_IF_ROW2 = unsafe { &raw const (*tabela.0).Table }.cast();
    // SAFETY: a tabela tem `NumEntries` linhas seguidas a partir de `Table`, e vive até o
    // `drop` de `tabela`, depois do último uso da fatia.
    let linhas = unsafe { std::slice::from_raw_parts(primeira, quantas) };
    Ok(linhas.iter().filter_map(interface).collect())
}

/// Uma linha da tabela, ou `None` para a de loopback e os filtros.
fn interface(linha: &MIB_IF_ROW2) -> Option<NetworkInterface> {
    let flags = linha.InterfaceAndOperStatusFlags._bitfield;
    if linha.Type == MIB_IF_TYPE_LOOPBACK || flags & FILTRO != 0 {
        return None;
    }
    let fim = linha
        .Alias
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(linha.Alias.len());
    Some(NetworkInterface {
        name: String::from_utf16_lossy(&linha.Alias[..fim]),
        received_bytes: linha.InOctets,
        sent_bytes: linha.OutOctets,
        up: linha.OperStatus == IfOperStatusUp,
        physical: flags & HARDWARE != 0,
    })
}

/// A tabela do `GetIfTable2`, que se solta com o `FreeMibTable`.
struct Tabela(*mut MIB_IF_TABLE2);

impl Drop for Tabela {
    fn drop(&mut self) {
        // SAFETY: o ponteiro veio de um `GetIfTable2` que deu certo e só é solto aqui.
        unsafe { FreeMibTable(self.0.cast()) };
    }
}
