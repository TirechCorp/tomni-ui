//! A janela oculta do Pageant: o caminho antigo (PuTTY até a 0.74, KiTTY, o plink de versões
//! antigas do TortoiseGit) acha uma janela de classe e título "Pageant" e lhe manda um
//! `WM_COPYDATA`, cujo texto é o nome de um file mapping onde está a mensagem do agente.
//! Conferido em `windows/pageant.c` do PuTTY 0.81.

use std::ffi::c_void;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, HANDLE, HINSTANCE, HLOCAL, HWND};
use windows::Win32::Foundation::{LPARAM, LRESULT, LocalFree, WPARAM};
use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
use windows::Win32::Security::{
    EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    TOKEN_QUERY, TokenOwner, TokenUser,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    FILE_MAP_ALL_ACCESS, FILE_MAP_WRITE, MEMORY_BASIC_INFORMATION, MapViewOfFile, OpenFileMappingA,
    UnmapViewOfFile, VirtualQuery,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW,
    GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, SetWindowLongPtrW, TranslateMessage, UnregisterClassW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_COPYDATA, WM_NCCREATE, WNDCLASSW,
};
use windows::core::{HSTRING, PCSTR, PCWSTR};

use crate::{Handle, io_error};

/// Diz se existe uma janela de classe `class` e título `title` (`FindWindowW`), a conferência
/// que o Pageant faz para saber se já há um agente (`agent_exists`).
pub fn window_exists(class: &str, title: &str) -> bool {
    let class = HSTRING::from(class);
    let title = HSTRING::from(title);
    // SAFETY: os dois textos são terminados em NUL e vivem durante a chamada, que só lê.
    unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr())) }.is_ok()
}

/// O que atende a mensagem: recebe a memória mapeada e diz se a atendeu.
type Handler = Box<dyn Fn(&mut [u8]) -> bool + Send>;

/// O que a janela precisa saber ao receber a mensagem.
struct State {
    id: usize,
    owners: Vec<OwnedSid>,
    handler: Handler,
}

/// Um SID copiado para memória nossa.
struct OwnedSid(Vec<u64>);

/// Os SIDs que o PuTTY aceita como dono do mapeamento: o usuário do token e o dono padrão dele
/// (`ourself` e `ourself2` do `pageant.c`).
fn accepted_owners() -> io::Result<Vec<OwnedSid>> {
    let mut token = HANDLE::default();
    // SAFETY: devolve o pseudo-handle do próprio processo, que não precisa ser fechado.
    let processo = unsafe { GetCurrentProcess() };
    // SAFETY: `processo` é um handle válido e `token` é um `HANDLE` válido onde a chamada grava
    // o handle novo.
    unsafe { OpenProcessToken(processo, TOKEN_QUERY, &mut token) }.map_err(io_error)?;
    let token = Handle(token);
    let mut donos = Vec::new();
    for classe in [TokenUser, TokenOwner] {
        let mut tamanho = 0u32;
        // SAFETY: sem buffer, só devolve o tamanho necessário em `tamanho`.
        let _ = unsafe { GetTokenInformation(token.0, classe, None, 0, &mut tamanho) };
        let mut buffer = vec![0u64; (tamanho as usize).div_ceil(8).max(1)];
        // SAFETY: `buffer` tem pelo menos `tamanho` bytes, que é o que se diz à chamada.
        unsafe {
            GetTokenInformation(
                token.0,
                classe,
                Some(buffer.as_mut_ptr().cast()),
                tamanho,
                &mut tamanho,
            )
        }
        .map_err(io_error)?;
        // O `TOKEN_USER` e o `TOKEN_OWNER` começam com um `PSID` que aponta para dentro do
        // próprio `buffer`: guarda-se o `buffer` todo (a memória do `Vec` não se mexe quando
        // ele muda de mãos) e o `sid_of` lê o ponteiro na hora de comparar.
        donos.push(OwnedSid(buffer));
    }
    Ok(donos)
}

impl State {
    /// O `WM_COPYDATA`: abre o mapa pelo nome, confere o dono, mapeia, chama o `handler` e
    /// desmapeia. Devolve se a mensagem foi atendida.
    fn serve(&self, dados: &COPYDATASTRUCT) -> bool {
        if dados.dwData != self.id || dados.lpData.is_null() || dados.cbData == 0 {
            return false;
        }
        // SAFETY: o remetente garante `cbData` bytes legíveis em `lpData` durante o
        // `WM_COPYDATA` (o sistema os copiou para este processo).
        let nome =
            unsafe { std::slice::from_raw_parts(dados.lpData.cast::<u8>(), dados.cbData as usize) };
        // Como o PuTTY: o texto tem de acabar em NUL dentro do que veio.
        if !nome.contains(&0) {
            return false;
        }
        // SAFETY: `nome` tem um NUL, então é um texto C válido que vive durante a chamada.
        let mapa = unsafe { OpenFileMappingA(FILE_MAP_ALL_ACCESS.0, false, PCSTR(nome.as_ptr())) };
        let Ok(mapa) = mapa else {
            return false;
        };
        let mapa = Handle(mapa);

        let mut dono = PSID::default();
        let mut descritor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `mapa` é um handle válido; `dono` e `descritor` são ponteiros válidos onde a
        // chamada grava o SID (dentro do `descritor`) e a descriptor, liberada abaixo.
        let erro = unsafe {
            GetSecurityInfo(
                mapa.0,
                SE_KERNEL_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut dono),
                None,
                None,
                None,
                Some(&mut descritor),
            )
        };
        if erro.is_err() {
            return false;
        }
        let aceito = self.owners.iter().any(|candidato| {
            let sid = sid_of(candidato);
            // SAFETY: os dois SIDs são válidos: o de `candidato` está no buffer dele e o `dono`
            // está dentro do `descritor`, ainda vivo.
            unsafe { EqualSid(sid, dono) }.is_ok()
        });
        // SAFETY: `descritor` foi alocado pelo `GetSecurityInfo` com `LocalAlloc` e `dono`, que
        // apontava para dentro dele, não é mais usado.
        let _ = unsafe { LocalFree(Some(HLOCAL(descritor.0))) };
        if !aceito {
            return false;
        }

        // SAFETY: `mapa` é um handle de file mapping válido, aberto com acesso total.
        let vista = unsafe { MapViewOfFile(mapa.0, FILE_MAP_WRITE, 0, 0, 0) };
        if vista.Value.is_null() {
            return false;
        }
        let mut info = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: `vista.Value` é o endereço de uma vista mapeada e `info` é uma estrutura válida
        // do tamanho que se diz à chamada.
        let medido = unsafe {
            VirtualQuery(
                Some(vista.Value.cast_const()),
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        let atendido = medido != 0 && {
            // SAFETY: a vista mapeia `RegionSize` bytes legíveis e graváveis a partir de
            // `vista.Value`, e ninguém mais usa esta fatia até o `UnmapViewOfFile` abaixo.
            let memoria = unsafe {
                std::slice::from_raw_parts_mut(vista.Value.cast::<u8>(), info.RegionSize)
            };
            (self.handler)(memoria)
        };
        // SAFETY: `vista` veio do `MapViewOfFile` acima e a fatia dela já não é usada.
        let _ = unsafe { UnmapViewOfFile(vista) };
        atendido
    }
}

/// O ponteiro do SID que o `OwnedSid` guarda no início do buffer (o primeiro campo do
/// `TOKEN_USER` e do `TOKEN_OWNER` é o `PSID`).
fn sid_of(sid: &OwnedSid) -> PSID {
    // SAFETY: o buffer foi preenchido por `GetTokenInformation` com um `TOKEN_USER` ou
    // `TOKEN_OWNER`, e os dois começam com um `PSID` (alinhado, pois o buffer é de `u64`).
    unsafe { *sid.0.as_ptr().cast::<PSID>() }
}

/// O procedimento da janela.
///
/// # Safety
///
/// Chamado só pelo sistema, com os argumentos da mensagem.
unsafe extern "system" fn procedimento(
    janela: HWND,
    mensagem: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match mensagem {
        WM_NCCREATE => {
            // SAFETY: no `WM_NCCREATE` o `lparam` aponta para um `CREATESTRUCTW` válido.
            let criacao = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            // SAFETY: `janela` é a janela que está sendo criada; guarda-se o ponteiro do
            // `State`, que a thread da janela mantém vivo até a janela morrer.
            unsafe { SetWindowLongPtrW(janela, GWLP_USERDATA, criacao.lpCreateParams as isize) };
            // SAFETY: a mensagem segue para o tratamento padrão, com os mesmos argumentos.
            unsafe { DefWindowProcW(janela, mensagem, wparam, lparam) }
        }
        WM_COPYDATA => {
            // SAFETY: `janela` é a nossa; o valor foi gravado no `WM_NCCREATE`.
            let estado = unsafe { GetWindowLongPtrW(janela, GWLP_USERDATA) } as *const State;
            if estado.is_null() || lparam.0 == 0 {
                return LRESULT(0);
            }
            // SAFETY: o ponteiro é o do `State` que a thread da janela mantém vivo, e só ela o usa.
            let estado = unsafe { &*estado };
            // SAFETY: o `lparam` do `WM_COPYDATA` aponta para um `COPYDATASTRUCT` válido durante
            // a mensagem.
            let dados = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
            // O pânico do `handler` não pode atravessar o Win32.
            let atendido = catch_unwind(AssertUnwindSafe(|| estado.serve(dados))).unwrap_or(false);
            LRESULT(isize::from(atendido))
        }
        WM_CLOSE => {
            // SAFETY: `janela` é a nossa e esta é a thread que a criou.
            let _ = unsafe { DestroyWindow(janela) };
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_DESTROY => {
            // SAFETY: encerra o laço de mensagens desta thread.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => {
            // SAFETY: o tratamento padrão, com os mesmos argumentos que o sistema passou.
            unsafe { DefWindowProcW(janela, mensagem, wparam, lparam) }
        }
    }
}

/// A janela oculta do Pageant, numa thread própria com o laço de mensagens.
///
/// Soltar fecha a janela e espera a thread.
pub struct CopyDataWindow {
    janela: isize,
    thread: Option<JoinHandle<()>>,
}

impl CopyDataWindow {
    /// Cria a janela de classe `class` e título `title`. Para cada `WM_COPYDATA` com `dwData ==
    /// id`, abre o file mapping cujo nome vem no texto, confere o dono (o usuário), mapeia e
    /// chama `handler` com a memória mapeada; o que o `handler` devolver é o resultado da
    /// mensagem (verdadeiro = atendida). O `handler` roda na thread da janela.
    pub fn start(
        class: &str,
        title: &str,
        id: usize,
        handler: impl Fn(&mut [u8]) -> bool + Send + 'static,
    ) -> io::Result<Self> {
        let estado = State {
            id,
            owners: accepted_owners()?,
            handler: Box::new(handler),
        };
        let class = class.to_owned();
        let title = title.to_owned();
        let (envia, recebe) = mpsc::channel::<io::Result<isize>>();
        let thread = std::thread::Builder::new()
            .name("pageant-janela".into())
            .spawn(move || run(&class, &title, estado, &envia))?;
        match recebe.recv() {
            Ok(Ok(janela)) => Ok(Self {
                janela,
                thread: Some(thread),
            }),
            Ok(Err(erro)) => {
                let _ = thread.join();
                Err(erro)
            }
            Err(_) => {
                let _ = thread.join();
                Err(io::Error::other(
                    "a thread da janela terminou antes de abrir",
                ))
            }
        }
    }
}

impl Drop for CopyDataWindow {
    fn drop(&mut self) {
        let janela = HWND(self.janela as *mut c_void);
        // SAFETY: postar uma mensagem em qualquer thread é permitido; se a janela já não
        // existe a chamada só falha.
        let _ = unsafe { PostMessageW(Some(janela), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// O corpo da thread: registra a classe, cria a janela, avisa quem a esperava e roda o laço.
fn run(class: &str, title: &str, estado: State, avisa: &mpsc::Sender<io::Result<isize>>) {
    let estado = Box::new(estado);
    let nome_da_classe = HSTRING::from(class);
    let titulo = HSTRING::from(title);

    // SAFETY: `None` pede o módulo do próprio executável.
    let modulo = match unsafe { GetModuleHandleW(PCWSTR::null()) } {
        Ok(modulo) => modulo,
        Err(erro) => {
            let _ = avisa.send(Err(io_error(erro)));
            return;
        }
    };
    let instancia = HINSTANCE(modulo.0);
    let classe = WNDCLASSW {
        lpfnWndProc: Some(procedimento),
        hInstance: instancia,
        lpszClassName: PCWSTR(nome_da_classe.as_ptr()),
        ..Default::default()
    };
    // SAFETY: `classe` é válida, o nome dela vive até o fim desta função e o procedimento é o
    // nosso, com a assinatura certa.
    if unsafe { RegisterClassW(&classe) } == 0 {
        let erro = windows::core::Error::from_thread();
        if erro.code() != ERROR_CLASS_ALREADY_EXISTS.to_hresult() {
            let _ = avisa.send(Err(io_error(erro)));
            return;
        }
    }
    let parametro = (&*estado as *const State).cast::<c_void>();
    // SAFETY: a classe está registrada; `parametro` aponta para o `State`, que vive nesta
    // thread até depois de a janela morrer. Sem pai, sem estilo e sem ser mostrada: a janela é
    // oculta, mas o `FindWindow` a acha.
    let janela = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(nome_da_classe.as_ptr()),
            PCWSTR(titulo.as_ptr()),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instancia),
            Some(parametro),
        )
    };
    let janela = match janela {
        Ok(janela) => janela,
        Err(erro) => {
            let _ = avisa.send(Err(io_error(erro)));
            // SAFETY: desfaz o registro feito acima.
            let _ = unsafe { UnregisterClassW(PCWSTR(nome_da_classe.as_ptr()), Some(instancia)) };
            return;
        }
    };
    let _ = avisa.send(Ok(janela.0 as isize));

    let mut mensagem = MSG::default();
    loop {
        // SAFETY: `mensagem` é uma estrutura válida; `None` recebe as mensagens desta thread.
        let ret = unsafe { GetMessageW(&mut mensagem, None, 0, 0) };
        if ret.0 <= 0 {
            break;
        }
        // SAFETY: `mensagem` foi preenchida pelo `GetMessageW` acima.
        let _ = unsafe { TranslateMessage(&mensagem) };
        // SAFETY: idem.
        unsafe { DispatchMessageW(&mensagem) };
    }
    // SAFETY: a janela já foi destruída; desfaz o registro da classe (falha sem problema se
    // outra janela ainda a usa).
    let _ = unsafe { UnregisterClassW(PCWSTR(nome_da_classe.as_ptr()), Some(instancia)) };
    drop(estado);
}
