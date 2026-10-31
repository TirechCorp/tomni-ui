//! O tráfego de cada interface de rede, para a aba Rede do app.

use std::ffi::CStr;
use std::io;

/// O tipo das interfaces Ethernet e Wi-Fi (`IFT_ETHER` do `net/if_types.h`, que a `libc` não
/// declara).
const ETHERNET: u8 = 0x06;
/// O tamanho do nome de uma interface com o NUL (`IFNAMSIZ` do `net/if.h`).
const TAMANHO_DO_NOME: usize = 16;
/// Quantas vezes pedir a lista de novo quando ela cresce entre o tamanho e a leitura.
const TENTATIVAS: usize = 4;

/// Uma interface de rede e os bytes que passaram por ela desde que ela subiu
/// ([`network_interfaces`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkInterface {
    /// O nome (`en0`, `utun3`).
    pub name: String,
    /// Os bytes recebidos.
    pub received_bytes: u64,
    /// Os bytes enviados.
    pub sent_bytes: u64,
    /// Se está ligada (`IFF_UP`).
    pub up: bool,
    /// Se é uma placa de verdade (Ethernet ou Wi-Fi), não uma VPN, ponte ou a rede ponto a ponto
    /// da Apple (`awdl`, `llw`).
    pub physical: bool,
}

/// As interfaces da máquina, sem a de loopback, pelo `sysctl` com `NET_RT_IFLIST2` (as
/// mensagens `RTM_IFINFO2`, com os contadores de 64 bits) e o nome pelo `if_indextoname`.
pub fn network_interfaces() -> io::Result<Vec<NetworkInterface>> {
    let mensagens = lista_das_interfaces()?;
    Ok(cabecalhos(&mensagens)
        .into_iter()
        .filter_map(|cabecalho| {
            let nome = nome_da_interface(u32::from(cabecalho.ifm_index))?;
            interface(nome, &cabecalho)
        })
        .collect())
}

/// Os bytes que o `sysctl` devolve: as mensagens de cada interface, uma atrás da outra.
fn lista_das_interfaces() -> io::Result<Vec<u8>> {
    let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
    for _ in 0..TENTATIVAS {
        let mut tamanho: libc::size_t = 0;
        // SAFETY: o `mib` tem os seis inteiros que o tamanho diz; sem buffer, o `sysctl` só
        // escreve o tamanho em `tamanho`.
        let erro = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as libc::c_uint,
                std::ptr::null_mut(),
                &mut tamanho,
                std::ptr::null_mut(),
                0,
            )
        };
        if erro != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut bytes = vec![0u8; tamanho];
        // SAFETY: o buffer tem `tamanho` bytes, que é o que o `sysctl` pode escrever.
        let erro = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as libc::c_uint,
                bytes.as_mut_ptr().cast(),
                &mut tamanho,
                std::ptr::null_mut(),
                0,
            )
        };
        if erro == 0 {
            bytes.truncate(tamanho);
            return Ok(bytes);
        }
        let falha = io::Error::last_os_error();
        // A lista cresceu entre as duas chamadas (uma interface nova): pede o tamanho de novo.
        if falha.raw_os_error() != Some(libc::ENOMEM) {
            return Err(falha);
        }
    }
    Err(io::Error::other(
        "a lista das interfaces não parou de crescer",
    ))
}

/// Os cabeçalhos `RTM_IFINFO2` das mensagens; as outras (os endereços) ficam de fora.
pub(crate) fn cabecalhos(bytes: &[u8]) -> Vec<libc::if_msghdr2> {
    let mut lista = Vec::new();
    let mut resto = bytes;
    // Toda mensagem começa com o tamanho (u16) e tem o tipo no quarto byte.
    while let [a, b, _, tipo, ..] = *resto {
        let tamanho = usize::from(u16::from_ne_bytes([a, b]));
        if tamanho == 0 || tamanho > resto.len() {
            break;
        }
        if i32::from(tipo) == libc::RTM_IFINFO2 && tamanho >= size_of::<libc::if_msghdr2>() {
            // SAFETY: a mensagem tem pelo menos o tamanho do `if_msghdr2`, que só tem inteiros
            // (qualquer valor é válido); a leitura sem alinhamento aceita qualquer endereço.
            lista.push(unsafe { std::ptr::read_unaligned(resto.as_ptr().cast()) });
        }
        resto = &resto[tamanho..];
    }
    lista
}

/// O nome da interface de índice `indice`, ou nada se ela sumiu.
fn nome_da_interface(indice: u32) -> Option<String> {
    let mut nome = [0 as libc::c_char; TAMANHO_DO_NOME];
    // SAFETY: o buffer tem os `IFNAMSIZ` bytes que o `if_indextoname` pode escrever.
    let achou = unsafe { libc::if_indextoname(indice, nome.as_mut_ptr()) };
    if achou.is_null() {
        return None;
    }
    // SAFETY: o `if_indextoname` deu certo e escreveu um nome terminado em NUL no buffer.
    let nome = unsafe { CStr::from_ptr(nome.as_ptr()) };
    Some(nome.to_string_lossy().into_owned())
}

/// A interface de um cabeçalho, ou nada se for a de loopback.
pub(crate) fn interface(name: String, cabecalho: &libc::if_msghdr2) -> Option<NetworkInterface> {
    let flags = cabecalho.ifm_flags;
    if flags & libc::IFF_LOOPBACK != 0 {
        return None;
    }
    let dados = cabecalho.ifm_data;
    let ponto_a_ponto_da_apple = name.starts_with("awdl") || name.starts_with("llw");
    Some(NetworkInterface {
        received_bytes: dados.ifi_ibytes,
        sent_bytes: dados.ifi_obytes,
        up: flags & libc::IFF_UP != 0,
        physical: dados.ifi_type == ETHERNET && !ponto_a_ponto_da_apple,
        name,
    })
}
