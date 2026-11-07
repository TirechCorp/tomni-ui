//! A saúde dos discos NVMe pelo plug-in `NVMeSMARTLib` do IOKit, sem root e sem o `smartctl`:
//! o registro de saúde do NVMe (a página de log 2, com 512 bytes), que quem chama interpreta.
//!
//! Cada `IOBlockStorageDevice` com a propriedade `NVMe SMART Capable` tem o plug-in nas
//! `IOCFPlugInTypes`. O `IOCreatePlugInInterfaceForService` dá o `IOCFPlugInInterface`, o
//! `QueryInterface` dele dá o `IONVMeSMARTInterface`, e o `SMARTReadData` deste preenche o
//! registro. Os discos SATA dos Macs com Intel ficam de fora (o plug-in deles é outro).

use std::ffi::{CStr, c_char, c_void};
use std::io;

use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};

use crate::disk::{
    IOIteratorNext, IOServiceGetMatchingServices, IOServiceMatching, Liberar, Objeto, propriedades,
    valor,
};

/// Um `CFUUIDRef` (os que se usam aqui são constantes, que não se soltam).
type Uuid = *const c_void;

/// Os 16 bytes de um UUID por valor (`CFUUIDBytes`, o `REFIID` do `QueryInterface`).
#[repr(C)]
#[derive(Clone, Copy)]
struct BytesDoUuid([u8; 16]);

/// O começo do `IOCFPlugInInterface` (o `IUNKNOWN_C_GUTS`).
#[repr(C)]
struct PlugIn {
    _reservado: *mut c_void,
    query_interface: unsafe extern "C" fn(*mut c_void, BytesDoUuid, *mut *mut c_void) -> i32,
    _add_ref: unsafe extern "C" fn(*mut c_void) -> u32,
    _release: unsafe extern "C" fn(*mut c_void) -> u32,
}

/// O começo do `IONVMeSMARTInterface`, até o `SMARTReadData`.
#[repr(C)]
struct Smart {
    _reservado: *mut c_void,
    _query_interface: unsafe extern "C" fn(*mut c_void, BytesDoUuid, *mut *mut c_void) -> i32,
    _add_ref: unsafe extern "C" fn(*mut c_void) -> u32,
    release: unsafe extern "C" fn(*mut c_void) -> u32,
    _version: u16,
    _revision: u16,
    smart_read_data: unsafe extern "C" fn(*mut c_void, *mut u8) -> libc::kern_return_t,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    #[allow(clippy::too_many_arguments)]
    fn CFUUIDGetConstantUUIDWithBytes(
        allocator: *const c_void,
        b0: u8,
        b1: u8,
        b2: u8,
        b3: u8,
        b4: u8,
        b5: u8,
        b6: u8,
        b7: u8,
        b8: u8,
        b9: u8,
        b10: u8,
        b11: u8,
        b12: u8,
        b13: u8,
        b14: u8,
        b15: u8,
    ) -> Uuid;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOCreatePlugInInterfaceForService(
        service: Objeto,
        plugin_type: Uuid,
        interface_type: Uuid,
        interface: *mut *mut *mut PlugIn,
        score: *mut i32,
    ) -> libc::kern_return_t;
    fn IODestroyPlugInInterface(interface: *mut *mut PlugIn) -> libc::kern_return_t;
    fn IORegistryEntrySearchCFProperty(
        entry: Objeto,
        plane: *const c_char,
        key: CFStringRef,
        allocator: *const c_void,
        options: u32,
    ) -> CFTypeRef;
}

/// `kIONVMeSMARTUserClientTypeID` (AA0FA6F9-C2D6-457F-B10B-59A13253292F).
const CLIENTE_SMART: [u8; 16] = [
    0xAA, 0x0F, 0xA6, 0xF9, 0xC2, 0xD6, 0x45, 0x7F, 0xB1, 0x0B, 0x59, 0xA1, 0x32, 0x53, 0x29, 0x2F,
];
/// `kIOCFPlugInInterfaceID` (C244E858-109C-11D4-91D4-0050E4C6426F).
const INTERFACE_PLUGIN: [u8; 16] = [
    0xC2, 0x44, 0xE8, 0x58, 0x10, 0x9C, 0x11, 0xD4, 0x91, 0xD4, 0x00, 0x50, 0xE4, 0xC6, 0x42, 0x6F,
];
/// `kIONVMeSMARTInterfaceID` (CCD1DB19-FD9A-4DAF-BF95-12454B230AB6).
const INTERFACE_SMART: [u8; 16] = [
    0xCC, 0xD1, 0xDB, 0x19, 0xFD, 0x9A, 0x4D, 0xAF, 0xBF, 0x95, 0x12, 0x45, 0x4B, 0x23, 0x0A, 0xB6,
];

const APARELHO: &CStr = c"IOBlockStorageDevice";
const PLANO: &CStr = c"IOService";
/// `kIORegistryIterateRecursively`: procura nos filhos, nos filhos deles e assim por diante.
const RECURSIVO: u32 = 1;
/// O tamanho do registro de saúde do NVMe.
const REGISTRO: usize = 512;

/// Um disco NVMe com SMART ([`nvme_health`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NvmeHealth {
    /// O nome BSD do disco inteiro (`disk0`), o mesmo do [`crate::DiskCounters`].
    pub name: String,
    /// O modelo (`Product Name` das `Device Characteristics`).
    pub model: Option<String>,
    /// O registro de saúde do NVMe (a página de log 2), com 512 bytes; `None` quando o plug-in
    /// não leu.
    pub smart_log: Option<Vec<u8>>,
}

/// Os discos NVMe com SMART deste computador, cada um com o registro de saúde. Não precisa de
/// root.
pub fn nvme_health() -> io::Result<Vec<NvmeHealth>> {
    // SAFETY: o nome termina em zero; a chamada devolve um dicionário novo (ou nulo).
    let busca = unsafe { IOServiceMatching(APARELHO.as_ptr()) };
    if busca.is_null() {
        return Err(io::Error::other("IOServiceMatching"));
    }
    let mut lista: Objeto = 0;
    // SAFETY: a busca é o dicionário válido de cima, que a chamada consome; `lista` é local.
    let erro = unsafe { IOServiceGetMatchingServices(0, busca, &mut lista) };
    if erro != 0 {
        return Err(io::Error::other(format!(
            "IOServiceGetMatchingServices: {erro}"
        )));
    }
    let lista = Liberar(lista);
    let mut discos = Vec::new();
    loop {
        // SAFETY: `lista` é um iterador válido até o `drop`; zero é o fim.
        let aparelho = unsafe { IOIteratorNext(lista.0) };
        if aparelho == 0 {
            break;
        }
        let aparelho = Liberar(aparelho);
        let Some(caracteristicas) = propriedades(aparelho.0) else {
            continue;
        };
        let com_smart = valor(&caracteristicas, "NVMe SMART Capable")
            .and_then(|v| v.downcast::<CFBoolean>())
            .is_some_and(bool::from);
        if !com_smart {
            continue;
        }
        let Some(nome) = nome_bsd(aparelho.0) else {
            continue;
        };
        let modelo = valor(&caracteristicas, "Device Characteristics")
            .and_then(|v| v.downcast::<CFDictionary>())
            .and_then(|c| valor(&c, "Product Name"))
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string().trim().to_owned())
            .filter(|s| !s.is_empty());
        discos.push(NvmeHealth {
            name: nome,
            model: modelo,
            smart_log: registro(aparelho.0),
        });
    }
    discos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(discos)
}

/// Se este computador é uma máquina virtual (`kern.hv_vmm_present`).
pub fn is_virtual_machine() -> bool {
    let mut presente: libc::c_int = 0;
    let mut tamanho = size_of::<libc::c_int>();
    // SAFETY: o nome termina em zero, a saída é o inteiro local com o tamanho dele, e não há
    // valor novo para gravar.
    let erro = unsafe {
        libc::sysctlbyname(
            c"kern.hv_vmm_present".as_ptr(),
            (&raw mut presente).cast(),
            &mut tamanho,
            std::ptr::null_mut(),
            0,
        )
    };
    erro == 0 && presente == 1
}

/// O nome BSD do primeiro disco abaixo do aparelho (`disk0`).
fn nome_bsd(aparelho: Objeto) -> Option<String> {
    let chave = CFString::from_static_string("BSD Name");
    // SAFETY: `aparelho` é válido, o plano termina em zero, a chave vive até o fim da chamada e
    // o alocador nulo é o padrão.
    let achado = unsafe {
        IORegistryEntrySearchCFProperty(
            aparelho,
            PLANO.as_ptr(),
            chave.as_concrete_TypeRef(),
            std::ptr::null(),
            RECURSIVO,
        )
    };
    if achado.is_null() {
        return None;
    }
    // SAFETY: a chamada criou a referência (a regra de criar): o `CFType` fica com ela.
    let achado = unsafe { CFType::wrap_under_create_rule(achado) };
    Some(achado.downcast::<CFString>()?.to_string())
}

/// Um UUID constante do Core Foundation.
fn uuid(b: [u8; 16]) -> Uuid {
    // SAFETY: só passa bytes; o UUID constante devolvido vive até o fim do processo.
    unsafe {
        CFUUIDGetConstantUUIDWithBytes(
            std::ptr::null(),
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15],
        )
    }
}

/// O `IOCFPlugInInterface` aberto, que se destrói ao sair.
struct PlugInAberto(*mut *mut PlugIn);

impl Drop for PlugInAberto {
    fn drop(&mut self) {
        // SAFETY: o plug-in veio do `IOCreatePlugInInterfaceForService` e só é destruído aqui.
        unsafe { IODestroyPlugInInterface(self.0) };
    }
}

/// O `IONVMeSMARTInterface`, que se solta ao sair.
struct SmartAberto(*mut *mut Smart);

impl SmartAberto {
    /// A tabela de funções da interface.
    fn tabela(&self) -> &Smart {
        // SAFETY: a interface veio do `QueryInterface` e vale até o `Release`, no `drop`.
        let tabela = unsafe { *self.0 };
        // SAFETY: a tabela da interface é estática no plug-in, que vive mais que `self`.
        unsafe { &*tabela }
    }
}

impl Drop for SmartAberto {
    fn drop(&mut self) {
        let release = self.tabela().release;
        // SAFETY: a interface é válida e o `QueryInterface` deu uma referência, solta só aqui.
        unsafe { release(self.0.cast()) };
    }
}

/// O registro de saúde do NVMe pelo plug-in do aparelho.
fn registro(aparelho: Objeto) -> Option<Vec<u8>> {
    let mut plugin: *mut *mut PlugIn = std::ptr::null_mut();
    let mut nota = 0i32;
    // SAFETY: `aparelho` é válido, os UUIDs são constantes e as saídas são locais.
    let erro = unsafe {
        IOCreatePlugInInterfaceForService(
            aparelho,
            uuid(CLIENTE_SMART),
            uuid(INTERFACE_PLUGIN),
            &mut plugin,
            &mut nota,
        )
    };
    if erro != 0 || plugin.is_null() {
        return None;
    }
    let plugin = PlugInAberto(plugin);
    // SAFETY: o plug-in é válido até o `drop` e aponta para a tabela de funções.
    let tabela = unsafe { *plugin.0 };
    if tabela.is_null() {
        return None;
    }
    // SAFETY: a tabela não é nula e vive tanto quanto o plug-in.
    let query_interface = unsafe { (*tabela).query_interface };
    let mut interface: *mut c_void = std::ptr::null_mut();
    // SAFETY: o plug-in é válido, o UUID vai por valor e a saída é local.
    let erro = unsafe {
        query_interface(
            plugin.0.cast(),
            BytesDoUuid(INTERFACE_SMART),
            &mut interface,
        )
    };
    if erro != 0 || interface.is_null() {
        return None;
    }
    // Solta antes do plug-in (as variáveis saem na ordem inversa).
    let smart = SmartAberto(interface.cast());
    let ler = smart.tabela().smart_read_data;
    let mut dados = vec![0u8; REGISTRO];
    // SAFETY: a interface é válida e o buffer local tem os 512 bytes do `NVMeSMARTData`.
    let erro = unsafe { ler(smart.0.cast(), dados.as_mut_ptr()) };
    (erro == 0).then_some(dados)
}
