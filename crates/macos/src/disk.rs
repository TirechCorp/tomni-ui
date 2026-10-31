//! As operações e os bytes de cada disco, para a aba Disco do app.

use std::ffi::{CStr, c_char, c_void};
use std::io;

use core_foundation::base::{CFType, TCFType, kCFAllocatorDefault};
use core_foundation::dictionary::{CFDictionary, CFMutableDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;

/// Um objeto do IOKit (`io_object_t`, `io_iterator_t`, `io_registry_entry_t`).
type Objeto = libc::mach_port_t;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingServices(
        main_port: libc::mach_port_t,
        matching: CFMutableDictionaryRef,
        existing: *mut Objeto,
    ) -> libc::kern_return_t;
    fn IOIteratorNext(iterator: Objeto) -> Objeto;
    fn IOObjectRelease(object: Objeto) -> libc::kern_return_t;
    fn IOObjectConformsTo(object: Objeto, class_name: *const c_char) -> libc::boolean_t;
    fn IORegistryEntryGetParentEntry(
        entry: Objeto,
        plane: *const c_char,
        parent: *mut Objeto,
    ) -> libc::kern_return_t;
    fn IORegistryEntryCreateCFProperties(
        entry: Objeto,
        properties: *mut CFMutableDictionaryRef,
        allocator: *const c_void,
        options: u32,
    ) -> libc::kern_return_t;
}

const MIDIA: &CStr = c"IOMedia";
const DRIVER: &CStr = c"IOBlockStorageDriver";
const CONTEINER_APFS: &CStr = c"AppleAPFSContainer";
const PLANO: &CStr = c"IOService";

/// Os contadores de um disco ([`disk_counters`]), desde o boot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiskCounters {
    /// O nome BSD (`disk0`).
    pub name: String,
    /// As leituras feitas.
    pub read_count: u64,
    /// As escritas feitas.
    pub write_count: u64,
    /// Os bytes lidos.
    pub read_bytes: u64,
    /// Os bytes escritos.
    pub written_bytes: u64,
    /// Um disco de verdade, e não o contêiner do APFS (que repete o tráfego do disco por baixo
    /// dele) nem uma imagem de disco montada.
    pub physical: bool,
}

/// Cada `IOMedia` inteiro (o filho de um `IOBlockStorageDriver`), com o dicionário
/// `Statistics` do driver: as operações e os bytes lidos e escritos.
pub fn disk_counters() -> io::Result<Vec<DiskCounters>> {
    // SAFETY: o nome termina em zero; a chamada devolve um dicionário novo (ou nulo).
    let busca = unsafe { IOServiceMatching(MIDIA.as_ptr()) };
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
        let midia = unsafe { IOIteratorNext(lista.0) };
        if midia == 0 {
            break;
        }
        let midia = Liberar(midia);
        if let Some(disco) = disco(midia.0) {
            discos.push(disco);
        }
    }
    Ok(discos)
}

/// O disco de uma mídia, se ela é o filho de um driver de bloco (um disco inteiro, não uma
/// partição).
fn disco(midia: Objeto) -> Option<DiskCounters> {
    let driver = Liberar(pai(midia)?);
    // SAFETY: `driver` é um objeto válido e o nome termina em zero.
    if unsafe { IOObjectConformsTo(driver.0, DRIVER.as_ptr()) } == 0 {
        return None;
    }
    let nome = valor(&propriedades(midia)?, "BSD Name")?
        .downcast::<CFString>()?
        .to_string();
    let estatisticas = valor(&propriedades(driver.0)?, "Statistics")?.downcast::<CFDictionary>()?;
    let numero = |chave| {
        valor(&estatisticas, chave)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or(0)
    };
    Some(DiskCounters {
        name: nome,
        read_count: numero("Operations (Read)"),
        write_count: numero("Operations (Write)"),
        read_bytes: numero("Bytes (Read)"),
        written_bytes: numero("Bytes (Write)"),
        physical: fisico(driver.0),
    })
}

/// Se o aparelho por baixo do driver é um disco de verdade: nem o contêiner do APFS nem uma
/// imagem (a "Physical Interconnect" das "Protocol Characteristics" diz "Virtual Interface").
fn fisico(driver: Objeto) -> bool {
    let Some(aparelho) = pai(driver).map(Liberar) else {
        return true;
    };
    // SAFETY: `aparelho` é um objeto válido e o nome termina em zero.
    if unsafe { IOObjectConformsTo(aparelho.0, CONTEINER_APFS.as_ptr()) } != 0 {
        return false;
    }
    let ligacao = propriedades(aparelho.0)
        .and_then(|p| valor(&p, "Protocol Characteristics"))
        .and_then(|v| v.downcast::<CFDictionary>())
        .and_then(|c| valor(&c, "Physical Interconnect"))
        .and_then(|v| v.downcast::<CFString>())
        .map(|s| s.to_string());
    eh_fisico(ligacao.as_deref())
}

/// A ligação de um aparelho é de um disco de verdade (sem ligação dita, conta como de verdade).
pub(crate) fn eh_fisico(ligacao: Option<&str>) -> bool {
    ligacao != Some("Virtual Interface")
}

/// O pai de `objeto` no plano dos serviços (que se solta com o `IOObjectRelease`).
fn pai(objeto: Objeto) -> Option<Objeto> {
    let mut pai: Objeto = 0;
    // SAFETY: `objeto` é válido, o plano termina em zero e `pai` é local.
    let erro = unsafe { IORegistryEntryGetParentEntry(objeto, PLANO.as_ptr(), &mut pai) };
    (erro == 0 && pai != 0).then_some(pai)
}

/// As propriedades de `objeto`, num dicionário novo.
fn propriedades(objeto: Objeto) -> Option<CFDictionary> {
    let mut dicionario: CFMutableDictionaryRef = std::ptr::null_mut();
    // SAFETY: `objeto` é válido e `dicionario` é local; o alocador padrão é uma constante.
    let erro = unsafe {
        IORegistryEntryCreateCFProperties(objeto, &mut dicionario, kCFAllocatorDefault.cast(), 0)
    };
    if erro != 0 || dicionario.is_null() {
        return None;
    }
    // SAFETY: a chamada criou o dicionário (a regra de criar): o `CFDictionary` fica com a
    // única referência e a solta no `drop`.
    Some(unsafe { CFDictionary::wrap_under_create_rule(dicionario) })
}

/// O valor de `chave` num dicionário.
fn valor(dicionario: &CFDictionary, chave: &'static str) -> Option<CFType> {
    let chave = CFString::from_static_string(chave);
    let item = dicionario.find(chave.as_CFTypeRef())?;
    // SAFETY: o valor é um objeto do Core Foundation que o dicionário guarda; a regra de pegar
    // soma uma referência, e o `CFType` a solta.
    Some(unsafe { CFType::wrap_under_get_rule(*item) })
}

/// Um objeto do IOKit que se solta ao sair.
struct Liberar(Objeto);

impl Drop for Liberar {
    fn drop(&mut self) {
        // SAFETY: o objeto veio de uma chamada que deu uma referência a quem chamou, e só é
        // solto aqui.
        unsafe { IOObjectRelease(self.0) };
    }
}
