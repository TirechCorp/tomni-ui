//! Os sensores de temperatura e o nome do processador, para a aba Sensores e o medidor do app.
//!
//! O macOS não tem API pública para as temperaturas. Os sensores do Apple Silicon aparecem como
//! serviços HID do fabricante (`PrimaryUsagePage` `0xff00`, `PrimaryUsage` 5), lidos pelo
//! `IOHIDEventSystemClient` do IOKit, sem root: as funções são exportadas pelo framework, mas
//! não estão nos cabeçalhos, e por isso são declaradas aqui. Num Mac com Intel a lista vem
//! vazia (os sensores de lá ficam no SMC).

use std::ffi::{CStr, c_void};
use std::io;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;

/// O tipo dos eventos de temperatura (`kIOHIDEventTypeTemperature`).
const EVENTO_TEMPERATURA: i64 = 15;
/// O campo do valor de um evento de temperatura (`IOHIDEventFieldBase(15)`).
const CAMPO_TEMPERATURA: i32 = 15 << 16;
/// A página e o uso dos sensores de temperatura do fabricante.
const PAGINA: i32 = 0xff00;
const USO: i32 = 5;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDEventSystemClientCreate(allocator: *const c_void) -> CFTypeRef;
    fn IOHIDEventSystemClientSetMatching(client: CFTypeRef, matching: CFDictionaryRef) -> i32;
    fn IOHIDEventSystemClientCopyServices(client: CFTypeRef) -> CFArrayRef;
    fn IOHIDServiceClientCopyProperty(service: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn IOHIDServiceClientCopyEvent(
        service: CFTypeRef,
        kind: i64,
        options: i32,
        timestamp: i64,
    ) -> CFTypeRef;
    fn IOHIDEventGetFloatValue(event: CFTypeRef, field: i32) -> f64;
}

/// Um sensor de temperatura ([`temperatures`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Temperature {
    /// O nome que o sistema dá (`PMU tdie1`, `NAND CH0 temp`); pode repetir.
    pub name: String,
    /// A temperatura, em graus Celsius.
    pub celsius: f64,
}

/// Os sensores de temperatura do Apple Silicon, na ordem do sistema. Vazio num Mac com Intel.
pub fn temperatures() -> io::Result<Vec<Temperature>> {
    // SAFETY: o alocador nulo é o padrão; a chamada devolve um cliente novo (ou nulo).
    let cliente = unsafe { IOHIDEventSystemClientCreate(std::ptr::null()) };
    if cliente.is_null() {
        return Err(io::Error::other("IOHIDEventSystemClientCreate"));
    }
    // SAFETY: a chamada criou o cliente (a regra de criar): o `CFType` fica com a referência.
    let cliente = unsafe { CFType::wrap_under_create_rule(cliente) };
    let busca = CFDictionary::from_CFType_pairs(&[
        (
            CFString::from_static_string("PrimaryUsagePage"),
            CFNumber::from(PAGINA),
        ),
        (
            CFString::from_static_string("PrimaryUsage"),
            CFNumber::from(USO),
        ),
    ]);
    // SAFETY: o cliente e o dicionário são válidos; a chamada não fica com eles.
    unsafe {
        IOHIDEventSystemClientSetMatching(cliente.as_CFTypeRef(), busca.as_concrete_TypeRef())
    };
    // SAFETY: o cliente é válido; a chamada devolve uma lista nova (ou nula, sem serviços).
    let servicos = unsafe { IOHIDEventSystemClientCopyServices(cliente.as_CFTypeRef()) };
    if servicos.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: a lista veio de uma chamada `Copy` (a regra de criar).
    let servicos: CFArray<CFType> = unsafe { CFArray::wrap_under_create_rule(servicos) };
    let produto = CFString::from_static_string("Product");
    let mut sensores = Vec::new();
    for servico in servicos.iter() {
        let Some(name) = nome(servico.as_CFTypeRef(), &produto) else {
            continue;
        };
        if let Some(celsius) = temperatura(servico.as_CFTypeRef()) {
            sensores.push(Temperature { name, celsius });
        }
    }
    Ok(sensores)
}

/// O `Product` de um serviço.
fn nome(servico: CFTypeRef, produto: &CFString) -> Option<String> {
    // SAFETY: o serviço é um item válido da lista e a chave é uma string válida; a chamada
    // devolve um valor novo (ou nulo).
    let valor = unsafe { IOHIDServiceClientCopyProperty(servico, produto.as_CFTypeRef()) };
    if valor.is_null() {
        return None;
    }
    // SAFETY: o valor veio de uma chamada `Copy` (a regra de criar).
    let valor = unsafe { CFType::wrap_under_create_rule(valor) };
    Some(valor.downcast::<CFString>()?.to_string())
}

/// A temperatura de um serviço, pelo evento mais novo dele.
fn temperatura(servico: CFTypeRef) -> Option<f64> {
    // SAFETY: o serviço é um item válido da lista; a chamada devolve um evento novo (ou nulo).
    let evento = unsafe { IOHIDServiceClientCopyEvent(servico, EVENTO_TEMPERATURA, 0, 0) };
    if evento.is_null() {
        return None;
    }
    // SAFETY: o evento veio de uma chamada `Copy` (a regra de criar).
    let evento = unsafe { CFType::wrap_under_create_rule(evento) };
    // SAFETY: o evento é válido e é de temperatura; o campo é o do valor dele.
    let celsius = unsafe { IOHIDEventGetFloatValue(evento.as_CFTypeRef(), CAMPO_TEMPERATURA) };
    celsius.is_finite().then_some(celsius)
}

/// O nome do processador (`machdep.cpu.brand_string`: "Apple M6").
pub fn cpu_brand() -> Option<String> {
    const NOME: &CStr = c"machdep.cpu.brand_string";
    let mut tamanho: libc::size_t = 0;
    // SAFETY: o nome termina em zero; sem buffer, a chamada só dá o tamanho em `tamanho`.
    let erro = unsafe {
        libc::sysctlbyname(
            NOME.as_ptr(),
            std::ptr::null_mut(),
            &mut tamanho,
            std::ptr::null_mut(),
            0,
        )
    };
    if erro != 0 || tamanho == 0 {
        return None;
    }
    let mut texto = vec![0u8; tamanho];
    // SAFETY: o buffer tem `tamanho` bytes, que a chamada não passa.
    let erro = unsafe {
        libc::sysctlbyname(
            NOME.as_ptr(),
            texto.as_mut_ptr().cast(),
            &mut tamanho,
            std::ptr::null_mut(),
            0,
        )
    };
    if erro != 0 {
        return None;
    }
    texto.truncate(tamanho);
    let nome = CStr::from_bytes_until_nul(&texto)
        .ok()?
        .to_str()
        .ok()?
        .trim();
    (!nome.is_empty()).then(|| nome.to_owned())
}
