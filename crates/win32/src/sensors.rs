//! Os sensores de temperatura, as ventoinhas e o nome do processador, para a aba Sensores e o
//! medidor do app, sem administrador.
//!
//! O Windows não tem uma API para a temperatura da CPU que funcione sem driver. O que se lê:
//!
//! - os discos, pelo `IOCTL_STORAGE_QUERY_PROPERTY` (`StorageDeviceTemperatureProperty`, com o
//!   modelo pelo `StorageDeviceProperty`), em cada `\\.\PhysicalDriveN` aberto sem direito de
//!   leitura;
//! - as zonas térmicas do ACPI, pelo contador `\Thermal Zone Information(*)\High Precision
//!   Temperature` do PDH (em décimos de kelvin); muitas placas de mesa não têm nenhuma;
//! - quando o LibreHardwareMonitor está aberto, os sensores dele pelo WMI
//!   (`root\LibreHardwareMonitor`, as classes `Sensor` e `Hardware`), que trazem a CPU, a
//!   memória, a placa-mãe, o vídeo e as ventoinhas; sem ele, o namespace não existe e a
//!   consulta volta vazia;
//! - o nome do processador, do registro
//!   (`HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0`, `ProcessorNameString`).

use std::collections::HashMap;
use std::mem::ManuallyDrop;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_PROPERTY_ID, STORAGE_PROPERTY_QUERY, STORAGE_TEMPERATURE_DATA_DESCRIPTOR,
    STORAGE_TEMPERATURE_INFO, StorageDeviceProperty, StorageDeviceTemperatureProperty,
};
use windows::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW,
};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0_0, VT_BSTR, VT_R4, VT_R8, VariantClear};
use windows::Win32::System::Wmi::{
    IEnumWbemClassObject, IWbemClassObject, IWbemLocator, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE, WbemLocator,
};
use windows::core::{BSTR, HSTRING, PCWSTR, w};

use crate::Handle;

/// Até que número de `\\.\PhysicalDriveN` se procura (o mesmo do `disk_counters`).
const DISCOS: u32 = 32;
/// Kelvin para Celsius.
const ZERO_ABSOLUTO: f64 = 273.15;

/// Um sensor de temperatura ([`sensors`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Temperature {
    /// De onde veio, no formato dos identificadores do LibreHardwareMonitor:
    /// `/storage/N` (o `\\.\PhysicalDriveN`), `/thermalzone/<instância>` (o ACPI) ou o
    /// `Identifier` do sensor no LibreHardwareMonitor (`/intelcpu/0/temperature/0`,
    /// `/lpc/nct6798d/temperature/1`, `/nvme/0/temperature/0`…).
    pub path: String,
    /// O nome do aparelho: o modelo do disco, a zona térmica ou o `Hardware` do
    /// LibreHardwareMonitor ("Intel Core i5-14600K").
    pub hardware: String,
    /// O nome do sensor ("CPU Package", "Core #1"); vazio quando o aparelho tem um só.
    pub name: String,
    /// A temperatura, em graus Celsius.
    pub celsius: f64,
    /// O limite de aviso do aparelho, quando ele diz (os discos).
    pub high: Option<f64>,
    /// O limite crítico do aparelho, quando ele diz (os discos).
    pub critical: Option<f64>,
}

/// Uma ventoinha do LibreHardwareMonitor ([`sensors`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Fan {
    /// O `Identifier` do sensor (`/lpc/nct6798d/fan/1`).
    pub path: String,
    /// O nome do aparelho.
    pub hardware: String,
    /// O nome da ventoinha ("Fan #2").
    pub name: String,
    /// As rotações por minuto.
    pub rpm: f64,
}

/// O que [`sensors`] achou.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sensors {
    /// As temperaturas: as do LibreHardwareMonitor, depois as dos discos e as das zonas
    /// térmicas.
    pub temperatures: Vec<Temperature>,
    /// As ventoinhas (só com o LibreHardwareMonitor).
    pub fans: Vec<Fan>,
}

/// Os sensores deste computador: os discos, as zonas térmicas e, com o LibreHardwareMonitor
/// aberto, os dele. O que falha fica de fora, sem erro.
pub fn sensors() -> Sensors {
    let mut sensores = monitor_de_hardware().unwrap_or_default();
    for n in 0..DISCOS {
        sensores.temperatures.extend(disco(n));
    }
    sensores.temperatures.extend(zonas_termicas());
    sensores
}

/// O nome do processador, do registro.
pub fn cpu_name() -> Option<String> {
    let mut texto = [0u16; 256];
    let mut tamanho = size_of_val(&texto) as u32;
    // SAFETY: as strings são constantes terminadas em zero; a saída é o buffer local, do
    // tamanho passado em `tamanho`.
    let erro = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"),
            w!("ProcessorNameString"),
            RRF_RT_REG_SZ,
            None,
            Some(texto.as_mut_ptr().cast()),
            Some(&mut tamanho),
        )
    };
    if erro.is_err() {
        return None;
    }
    let fim = texto.iter().position(|&c| c == 0).unwrap_or(texto.len());
    let nome = String::from_utf16_lossy(&texto[..fim]).trim().to_owned();
    (!nome.is_empty()).then_some(nome)
}

// --- Os discos ---

/// As temperaturas do disco `n` (uma por sensor dele), com o modelo e os limites.
fn disco(n: u32) -> Vec<Temperature> {
    let caminho = HSTRING::from(format!(r"\\.\PhysicalDrive{n}"));
    // SAFETY: o caminho é uma string terminada em zero que vive até o fim da chamada; o acesso
    // zero só deixa consultar, e o `Handle` fecha o que abrir.
    let aberto = unsafe {
        CreateFileW(
            &caminho,
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None::<HANDLE>,
        )
    };
    let Ok(aberto) = aberto else {
        return Vec::new();
    };
    let aberto = Handle(aberto);
    let Some(dados) = propriedade(aberto.0, StorageDeviceTemperatureProperty, 1024) else {
        return Vec::new();
    };
    if size_of_val(dados.as_slice()) < size_of::<STORAGE_TEMPERATURE_DATA_DESCRIPTOR>() {
        return Vec::new();
    }
    // SAFETY: o buffer tem pelo menos o tamanho do descritor e o alinhamento de `u64`; a
    // leitura sem alinhamento copia o valor.
    let descritor = unsafe {
        std::ptr::read_unaligned(dados.as_ptr().cast::<STORAGE_TEMPERATURE_DATA_DESCRIPTOR>())
    };
    let modelo = modelo(aberto.0).unwrap_or_else(|| format!("PhysicalDrive{n}"));
    let limite = |graus: i16| (graus > 0).then_some(f64::from(graus));
    let deslocamento = std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, TemperatureInfo);
    let quantos = usize::from(descritor.InfoCount);
    let mut temperaturas = Vec::new();
    for i in 0..quantos {
        let inicio = deslocamento + i * size_of::<STORAGE_TEMPERATURE_INFO>();
        if inicio + size_of::<STORAGE_TEMPERATURE_INFO>() > size_of_val(dados.as_slice()) {
            break;
        }
        let item = em_bytes(&dados)[inicio..].as_ptr();
        // SAFETY: o item cabe no buffer (conferido acima); a leitura sem alinhamento copia.
        let info = unsafe { std::ptr::read_unaligned(item.cast::<STORAGE_TEMPERATURE_INFO>()) };
        temperaturas.push(Temperature {
            path: format!("/storage/{n}"),
            hardware: modelo.clone(),
            name: if quantos > 1 && info.Index > 0 {
                format!("#{}", info.Index)
            } else {
                String::new()
            },
            celsius: f64::from(info.Temperature),
            high: limite(descritor.WarningTemperature),
            critical: limite(descritor.CriticalTemperature),
        });
    }
    temperaturas
}

/// O modelo do disco (o `ProductId` do `STORAGE_DEVICE_DESCRIPTOR`).
fn modelo(disco: HANDLE) -> Option<String> {
    let dados = propriedade(disco, StorageDeviceProperty, 1024)?;
    if size_of_val(dados.as_slice()) < size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return None;
    }
    // SAFETY: o buffer tem pelo menos o tamanho do descritor; a leitura sem alinhamento copia.
    let descritor =
        unsafe { std::ptr::read_unaligned(dados.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>()) };
    let bytes: &[u8] = em_bytes(&dados);
    let inicio = usize::try_from(descritor.ProductIdOffset).ok()?;
    if inicio == 0 || inicio >= bytes.len() {
        return None;
    }
    let resto = &bytes[inicio..];
    let fim = resto.iter().position(|&b| b == 0).unwrap_or(resto.len());
    let nome = String::from_utf8_lossy(&resto[..fim]).trim().to_owned();
    (!nome.is_empty()).then_some(nome)
}

/// Os bytes de um buffer de `u64`.
fn em_bytes(dados: &[u64]) -> &[u8] {
    // SAFETY: um `u64` é feito de 8 bytes sem preenchimento; o slice de bytes cobre a mesma
    // memória, com o mesmo tempo de vida.
    unsafe { std::slice::from_raw_parts(dados.as_ptr().cast::<u8>(), size_of_val(dados)) }
}

/// A resposta de um `IOCTL_STORAGE_QUERY_PROPERTY` padrão, num buffer de `u64` (para o
/// alinhamento) de até `bytes`.
fn propriedade(disco: HANDLE, id: STORAGE_PROPERTY_ID, bytes: usize) -> Option<Vec<u64>> {
    let pedido = STORAGE_PROPERTY_QUERY {
        PropertyId: id,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut saida = vec![0u64; bytes.div_ceil(8)];
    let mut escritos = 0u32;
    // SAFETY: a entrada é o pedido local, do tamanho passado; a saída é o buffer local, do
    // tamanho passado; `escritos` é local; a chamada é síncrona (sem `OVERLAPPED`).
    unsafe {
        DeviceIoControl(
            disco,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some((&raw const pedido).cast()),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(saida.as_mut_ptr().cast()),
            size_of_val(saida.as_slice()) as u32,
            Some(&mut escritos),
            None,
        )
    }
    .ok()?;
    saida.truncate((escritos as usize).div_ceil(8));
    Some(saida)
}

// --- As zonas térmicas do ACPI ---

/// Uma consulta do PDH que se fecha ao ser solta.
struct Consulta(PDH_HQUERY);

impl Drop for Consulta {
    fn drop(&mut self) {
        // SAFETY: a consulta veio do `PdhOpenQueryW` e só é fechada aqui.
        unsafe { PdhCloseQuery(self.0) };
    }
}

/// As zonas térmicas do contador do PDH.
fn zonas_termicas() -> Vec<Temperature> {
    let mut consulta = PDH_HQUERY::default();
    // SAFETY: sem fonte de dados (o computador ao vivo); a saída é local.
    if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut consulta) } != 0 {
        return Vec::new();
    }
    let consulta = Consulta(consulta);
    let mut contador = PDH_HCOUNTER::default();
    // SAFETY: a consulta é válida, o caminho é uma constante terminada em zero e a saída é
    // local.
    let erro = unsafe {
        PdhAddEnglishCounterW(
            consulta.0,
            w!(r"\Thermal Zone Information(*)\High Precision Temperature"),
            0,
            &mut contador,
        )
    };
    if erro != 0 {
        return Vec::new();
    }
    // SAFETY: a consulta é válida e tem o contador.
    if unsafe { PdhCollectQueryData(consulta.0) } != 0 {
        return Vec::new();
    }
    let mut tamanho = 0u32;
    let mut quantos = 0u32;
    // SAFETY: sem buffer, a chamada só dá o tamanho e a quantidade, nas saídas locais.
    let erro = unsafe {
        PdhGetFormattedCounterArrayW(contador, PDH_FMT_DOUBLE, &mut tamanho, &mut quantos, None)
    };
    if erro != PDH_MORE_DATA || quantos == 0 {
        return Vec::new();
    }
    // Os itens e, depois deles, os nomes, no mesmo buffer (de `u64`, para o alinhamento).
    let mut buffer = vec![0u64; (tamanho as usize).div_ceil(8)];
    // SAFETY: o buffer tem pelo menos `tamanho` bytes, que a chamada não passa.
    let erro = unsafe {
        PdhGetFormattedCounterArrayW(
            contador,
            PDH_FMT_DOUBLE,
            &mut tamanho,
            &mut quantos,
            Some(buffer.as_mut_ptr().cast()),
        )
    };
    if erro != 0 {
        return Vec::new();
    }
    let quantos = (quantos as usize)
        .min(size_of_val(buffer.as_slice()) / size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>());
    // SAFETY: a chamada pôs `quantos` itens no começo do buffer, alinhado a 8 (o alinhamento do
    // item), e o slice não passa do buffer, que vive até o fim da função.
    let itens = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
            quantos,
        )
    };
    let mut zonas = Vec::new();
    for item in itens {
        if item.FmtValue.CStatus != 0 {
            continue;
        }
        // SAFETY: com `PDH_FMT_DOUBLE` e o estado bom, o valor é o `doubleValue`.
        let decimos_de_kelvin = unsafe { item.FmtValue.Anonymous.doubleValue };
        // SAFETY: o nome aponta para dentro do buffer, terminado em zero.
        let instancia = unsafe { item.szName.to_string() }.unwrap_or_default();
        zonas.push(Temperature {
            path: format!("/thermalzone/{instancia}"),
            hardware: instancia,
            name: String::new(),
            celsius: decimos_de_kelvin / 10.0 - ZERO_ABSOLUTO,
            high: None,
            critical: None,
        });
    }
    zonas
}

// --- O LibreHardwareMonitor, pelo WMI ---

/// O COM desta thread, solto ao sair se esta chamada o iniciou.
struct Com(bool);

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: o `CoInitializeEx` desta thread deu certo e é desfeito uma vez.
            unsafe { CoUninitialize() };
        }
    }
}

/// Os sensores do LibreHardwareMonitor; `None` sem ele (o namespace não existe).
fn monitor_de_hardware() -> Option<Sensors> {
    // SAFETY: sem reservado; o modo da thread é o multithread. `S_FALSE` (já iniciado) também
    // pede o `CoUninitialize`; um modo diferente (`RPC_E_CHANGED_MODE`) não.
    let iniciado = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let _com = Com(iniciado.is_ok());
    // SAFETY: o CLSID é a constante do WMI; o objeto é criado neste processo.
    let localizador: IWbemLocator =
        unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let vazio = BSTR::new();
    // SAFETY: as strings são `BSTR` válidos até o fim da chamada; sem contexto.
    let servicos = unsafe {
        localizador.ConnectServer(
            &BSTR::from(r"root\LibreHardwareMonitor"),
            &vazio,
            &vazio,
            &vazio,
            0,
            &vazio,
            None,
        )
    }
    .ok()?;
    // SAFETY: o proxy é o serviço de cima; as constantes são as do WMI local.
    unsafe {
        CoSetProxyBlanket(
            &servicos,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
    }
    .ok()?;
    let consultar = |wql: &str| -> Option<IEnumWbemClassObject> {
        // SAFETY: as strings são `BSTR` válidos até o fim da chamada; sem contexto.
        unsafe {
            servicos.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(wql),
                WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0),
                None,
            )
        }
        .ok()
    };
    let mut aparelhos = HashMap::new();
    for objeto in objetos(consultar("SELECT Identifier, Name FROM Hardware")?) {
        if let (Some(id), Some(nome)) =
            (texto(&objeto, w!("Identifier")), texto(&objeto, w!("Name")))
        {
            aparelhos.insert(id, nome);
        }
    }
    let mut sensores = Sensors::default();
    let wql = "SELECT Identifier, Name, Parent, SensorType, Value FROM Sensor \
               WHERE SensorType = 'Temperature' OR SensorType = 'Fan'";
    for objeto in objetos(consultar(wql)?) {
        let (Some(path), Some(name), Some(tipo), Some(valor)) = (
            texto(&objeto, w!("Identifier")),
            texto(&objeto, w!("Name")),
            texto(&objeto, w!("SensorType")),
            numero(&objeto, w!("Value")),
        ) else {
            continue;
        };
        let hardware = texto(&objeto, w!("Parent"))
            .and_then(|pai| aparelhos.get(&pai).cloned())
            .unwrap_or_default();
        if tipo == "Fan" {
            sensores.fans.push(Fan {
                path,
                hardware,
                name,
                rpm: valor,
            });
        } else {
            sensores.temperatures.push(Temperature {
                path,
                hardware,
                name,
                celsius: valor,
                high: None,
                critical: None,
            });
        }
    }
    Some(sensores)
}

/// Os objetos de uma consulta, um de cada vez, até o fim.
fn objetos(lista: IEnumWbemClassObject) -> impl Iterator<Item = IWbemClassObject> {
    std::iter::from_fn(move || {
        let mut um = [None];
        let mut devolvidos = 0u32;
        // SAFETY: a saída é o vetor local de um item e o contador local.
        let erro = unsafe { lista.Next(WBEM_INFINITE, &mut um, &mut devolvidos) };
        if erro.is_err() || devolvidos == 0 {
            return None;
        }
        um[0].take()
    })
}

/// Um `VARIANT` que se limpa ao ser solto.
struct Valor(VARIANT);

impl Drop for Valor {
    fn drop(&mut self) {
        // SAFETY: o `VARIANT` veio do `Get` (ou é o vazio) e é limpo uma vez.
        let _ = unsafe { VariantClear(&mut self.0) };
    }
}

/// A propriedade `nome` de um objeto do WMI.
fn propriedade_wmi(objeto: &IWbemClassObject, nome: PCWSTR) -> Option<Valor> {
    let mut valor = Valor(VARIANT::default());
    // SAFETY: o nome é uma constante terminada em zero e a saída é o `VARIANT` local.
    unsafe { objeto.Get(nome, 0, &mut valor.0, None, None) }.ok()?;
    Some(valor)
}

/// O corpo de um `VARIANT`: o tipo (`vt`) e o valor.
fn corpo(valor: &Valor) -> &VARIANT_0_0 {
    // SAFETY: a forma com o `vt` é a de todo `VARIANT` (o `DECIMAL` da outra forma também
    // começa pelo `vt`), e o `Get` o preencheu (ou ele é o vazio, `VT_EMPTY`).
    unsafe { &valor.0.Anonymous.Anonymous }
}

/// Uma propriedade de texto.
fn texto(objeto: &IWbemClassObject, nome: PCWSTR) -> Option<String> {
    let valor = propriedade_wmi(objeto, nome)?;
    let corpo = corpo(&valor);
    if corpo.vt != VT_BSTR {
        return None;
    }
    // SAFETY: com o tipo `VT_BSTR`, o valor é o `bstrVal`, que continua do `Valor` (o
    // `ManuallyDrop` não o solta aqui).
    let bstr: &ManuallyDrop<BSTR> = unsafe { &corpo.Anonymous.bstrVal };
    Some(bstr.to_string())
}

/// Uma propriedade de número (o `Value` dos sensores é `real32`).
fn numero(objeto: &IWbemClassObject, nome: PCWSTR) -> Option<f64> {
    let valor = propriedade_wmi(objeto, nome)?;
    let corpo = corpo(&valor);
    let numero = if corpo.vt == VT_R4 {
        // SAFETY: com o tipo `VT_R4`, o valor é o `fltVal`.
        f64::from(unsafe { corpo.Anonymous.fltVal })
    } else if corpo.vt == VT_R8 {
        // SAFETY: com o tipo `VT_R8`, o valor é o `dblVal`.
        unsafe { corpo.Anonymous.dblVal }
    } else {
        return None;
    };
    numero.is_finite().then_some(numero)
}
