//! Integração com a plataforma via `winit`: janela, input, DPI, IME e superfície
//! em Windows, Linux, macOS e Android.
//!
//! O [`run`] abre a janela, renderiza na GPU com o `vello` e repassa os eventos a um
//! [`Handler`]. A fachada `tomni-ui` implementa o `Handler` para o `Ui`; este crate não
//! depende dela.
//!
//! O que a plataforma resolve aqui, e não no core:
//!
//! - **Cliques:** o `winit` entrega cada clique isolado; o [`ClickCounter`] conta o duplo e o
//!   triplo.
//! - **Pisca do cursor:** o [`Blink`] alterna o cursor a cada 530 ms enquanto há um campo com
//!   foco, pelo `ControlFlow::WaitUntil` do `winit`. Na janela sem o foco do sistema, o cursor
//!   fica apagado e parado.
//! - **Quadros:** a janela só pinta quando um evento, um prazo do [`Handler`] ou o pisca muda
//!   a tela. Uma superfície que falha é refeita uma vez; se falhar de novo, o quadro espera o
//!   próximo evento, sem laço. Com `TOMNI_UI_TRACE_FRAMES=1`, cada janela escreve no stderr,
//!   uma vez por segundo em que pintou, quantos quadros foram e por quê (evento, prazo, pisca,
//!   superfície ou sistema) e, nos de prazo, quais widgets animam.
//! - **Rede de segurança:** uma janela que pinta mais de 20 quadros por segundo, sem parar, por
//!   5 s sem nenhuma entrada da pessoa (tecla, ponteiro, roda, toque, IME, arquivos soltos,
//!   tamanho, foco) é um defeito. O runner limita o ritmo dela a 10 quadros por segundo (100 ms
//!   entre um e outro, pelo `ControlFlow::WaitUntil`, sem dormir a thread; o pedido que chega
//!   cedo fica retido e sai no horário) e escreve no stderr, uma vez por episódio, mesmo sem o
//!   rastro: `tomni-ui: Janela(0) pintou 60 quadros/s por 5 s sem entrada; limitando a 10/s
//!   (motivos: superfície 60)`, com os widgets do prazo quando há. A primeira entrada devolve o
//!   ritmo normal na hora. A saída de um terminal, o `wake()` de outra thread, os prazos e o
//!   pisca não contam como entrada: o terminal que despeja texto sem ninguém mexer também é
//!   limitado, e continua legível a 10 quadros por segundo.
//! - **DPI:** o `Ui` trabalha em pixels lógicos e a cena é escalada pelo fator da janela.
//! - **Clipboard:** o do sistema no desktop; no Android, um em memória.
//! - **Barra de título:** com [`WindowOptions::title_bar`], a janela abre sem a moldura do
//!   sistema (no macOS, com a barra transparente e os três botões do sistema por cima) e o
//!   runner atende os pedidos da `TitleBar` ([`Handler::window_requests`]): arrastar,
//!   redimensionar pelas bordas, maximizar, minimizar e fechar. Se o sistema recusar o arrasto,
//!   a moldura volta ([`WindowChrome::System`]). No Android, a do sistema fica sempre.
//! - **Mica:** com [`Backdrop::Mica`], no Windows 11, o `window-vibrancy` liga o fundo do
//!   sistema e a janela fica transparente; nos outros sistemas, o fundo continua sólido.
//!
//! # Android
//!
//! Com `NativeActivity`, o `winit` 0.30 só entrega teclas simples do teclado virtual: não trata
//! o `TextEvent` do `GameActivity`, então composição (IME) e acentos não chegam. A safe area
//! vem do `content_rect` da janela, consultado a cada redimensionamento e redesenho, porque o
//! `winit` não avisa quando ela muda.

mod blink;
mod clicks;
mod clipboard;
#[cfg(not(target_os = "android"))]
mod crash;
#[cfg(not(target_os = "android"))]
mod file_dialog;
mod frames;
mod icon;
pub mod keys;
mod launcher;
#[cfg(not(target_os = "android"))]
mod placement;
mod render_guard;
mod runner;
#[cfg(not(target_os = "android"))]
mod tray;

use std::fmt;
use std::time::Duration;

use tomni_ui_core::kurbo::{Point, Vec2};
use tomni_ui_core::{
    Clipboard, Cursor, DragData, Ime, Insets, Key, Modifiers, PointerKind, ResizeEdge, TextInput,
    WindowChrome, WindowRequest,
};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, GpuError, vello};

pub use blink::Blink;
pub use clicks::ClickCounter;
pub use clipboard::system as system_clipboard;
#[cfg(not(target_os = "android"))]
pub use crash::{CrashReport, alert};
pub use icon::{WindowIcon, rgba};
pub use launcher::{LauncherEntry, LauncherState};
pub use runner::{run, run_windows};
pub use winit;

/// `AndroidApp` recebido em `android_main`, que o [`run`] precisa no Android.
#[cfg(target_os = "android")]
pub use winit::platform::android::activity::AndroidApp;

/// O que roda dentro da janela. Posições e tamanhos em pixels lógicos.
///
/// Depois de cada evento o [`run`] chama [`Handler::frame`] e, se ele retornar `true`, pede um
/// redesenho, em que chama [`Handler::render`].
pub trait Handler {
    /// Tamanho da área da janela.
    fn resize(&mut self, width: u32, height: u32);

    /// Área ocupada pelo sistema (barra de status, entalhe, teclado virtual).
    fn set_safe_area(&mut self, insets: Insets);

    /// Tema do sistema: escuro ou claro.
    fn set_dark(&mut self, dark: bool);

    /// O app fixou o tema (`Some(true)` é o escuro) ou voltou a seguir o sistema (`None`, e o
    /// [`Handler::set_dark`] vem logo depois com o tema de agora).
    fn set_theme_override(&mut self, dark: Option<bool>) {
        let _ = dark;
    }

    /// Área de transferência do sistema.
    fn set_clipboard(&mut self, clipboard: Box<dyn Clipboard>);

    /// Ponteiro em `point`.
    fn pointer_move(&mut self, point: Point, kind: PointerKind);

    /// Botão primário ou dedo pressionado. `clicks` é 1, 2 (duplo) ou 3 (triplo).
    fn pointer_down(&mut self, point: Point, kind: PointerKind, clicks: u8);

    /// Botão primário ou dedo solto.
    fn pointer_up(&mut self, point: Point, kind: PointerKind);

    /// Botão secundário (o direito) pressionado em `point`.
    fn secondary_down(&mut self, point: Point) {
        let _ = point;
    }

    /// Botão secundário (o direito) solto em `point`.
    fn secondary_up(&mut self, point: Point) {
        let _ = point;
    }

    /// Botão do meio pressionado em `point`.
    fn middle_down(&mut self, point: Point) {
        let _ = point;
    }

    /// Botão do meio solto em `point`.
    fn middle_up(&mut self, point: Point) {
        let _ = point;
    }

    /// Arquivos do sistema (do gerenciador de arquivos) soltos sobre a janela, com o ponteiro em
    /// `point`. Devolve se algum alvo aceitou. Não chega no Wayland do winit.
    fn files_dropped(&mut self, paths: Vec<std::path::PathBuf>, point: Point) -> bool {
        let _ = (paths, point);
        false
    }

    /// O ponteiro saiu da janela ou o toque foi cancelado.
    fn pointer_cancel(&mut self);

    /// Roda do mouse ou rolagem do touchpad em `point`, em pixels: `delta` positivo mostra o
    /// que está mais abaixo (ou à direita).
    fn wheel(&mut self, point: Point, delta: Vec2);

    /// Modificadores pressionados agora.
    fn set_modifiers(&mut self, modifiers: Modifiers);

    /// Tecla pressionada (também nas repetições).
    fn key_down(&mut self, key: Key);

    /// Tecla solta.
    fn key_up(&mut self, key: Key);

    /// Texto digitado, já com o layout do teclado e as teclas mortas aplicados.
    fn type_text(&mut self, text: &str);

    /// Composição do IME.
    fn ime(&mut self, ime: Ime);

    /// Fase do pisca do cursor.
    fn set_caret_visible(&mut self, visible: bool);

    /// Roda o frame. Retorna se a tela precisa ser pintada de novo.
    fn frame(&mut self) -> bool;

    /// Pinta a tela.
    fn render(&self, canvas: &mut dyn Canvas);

    /// Cor de fundo, para onde nada for pintado.
    fn background(&self) -> Color;

    /// Título da janela.
    fn title(&self) -> Option<String>;

    /// Entrada de texto pedida pelo widget com foco: liga o IME e o pisca do cursor.
    fn text_input(&mut self) -> Option<TextInput>;

    /// Passa `dt` de tempo (a contagem da dica). Só é chamado enquanto há
    /// [`Handler::deadline`].
    fn advance(&mut self, dt: Duration) {
        let _ = dt;
    }

    /// Quanto falta para algo mudar sozinho na tela (uma dica abrir): o loop acorda a tempo
    /// e chama [`Handler::advance`].
    fn deadline(&self) -> Option<Duration> {
        None
    }

    /// O que o [`Handler::deadline`] espera, em texto (os widgets que animam, a dica), para o
    /// rastro dos quadros (`TOMNI_UI_TRACE_FRAMES`) e o aviso da janela que pinta sem parar.
    /// Só é chamado com o rastro ligado ou quando o aviso sai.
    #[doc(hidden)]
    fn deadline_reason(&self) -> Option<String> {
        None
    }

    /// Pedidos à janela feitos desde a última chamada (os da `TitleBar`). O runner atende
    /// depois de cada evento.
    fn window_requests(&mut self) -> Vec<WindowRequest> {
        Vec::new()
    }

    /// A borda da janela em `point` que redimensiona, para o cursor mostrar as setas.
    fn resize_edge(&self, point: Point) -> Option<ResizeEdge> {
        let _ = point;
        None
    }

    /// Forma do ponteiro em `point`, fora das bordas que redimensionam: a do widget embaixo
    /// (ou do que segura o ponteiro, como o divisor sendo arrastado).
    fn cursor(&self, point: Point) -> Cursor {
        let _ = point;
        Cursor::Default
    }

    /// A janela ganhou (`true`) ou perdeu o foco do sistema.
    fn set_window_active(&mut self, active: bool) {
        let _ = active;
    }

    /// Se a tela tem uma `TitleBar`: a janela abre sem a moldura do sistema. O runner pergunta
    /// ao abrir uma janela pedida por `open_window`.
    fn has_title_bar(&self) -> bool {
        false
    }

    /// O item que está sendo arrastado a partir desta janela (uma aba), depois de passar do
    /// limiar. O runner o leva às outras janelas do processo.
    fn drag_data(&self) -> Option<DragData> {
        None
    }

    /// Um item arrastado a partir de outra janela passou por `point` desta. Devolve se algum
    /// alvo aceita.
    fn foreign_drag_over(&mut self, data: &DragData, point: Point) -> bool {
        let _ = (data, point);
        false
    }

    /// O item arrastado a partir de outra janela saiu desta.
    fn foreign_drag_leave(&mut self) {}

    /// Um item arrastado a partir de outra janela foi solto em `point` desta. Devolve se algum
    /// alvo aceitou.
    fn foreign_drop(&mut self, data: DragData, point: Point) -> bool {
        let _ = (data, point);
        false
    }

    /// Cancela o arrasto que começou nesta janela, porque o item foi para outra.
    fn cancel_drag(&mut self) {}

    /// O arrasto que começou nesta janela foi solto fora de todas as janelas, em `at` na tela
    /// (`None` onde o sistema não informa a posição). Devolve se havia arrasto.
    fn drop_outside(&mut self, at: Option<(i32, i32)>) -> bool {
        let _ = at;
        false
    }

    /// A janela foi maximizada (`true`) ou restaurada.
    fn set_maximized(&mut self, maximized: bool) {
        let _ = maximized;
    }

    /// Quem desenha a moldura: o runner decide ao abrir a janela e muda para
    /// [`WindowChrome::System`] se o sistema recusar o arrasto.
    fn set_chrome(&mut self, chrome: WindowChrome) {
        let _ = chrome;
    }

    /// A janela ficou transparente, com o Mica do sistema atrás: o fundo não é mais pintado
    /// ([`Handler::background`] passa a transparente).
    fn set_transparent(&mut self, transparent: bool) {
        let _ = transparent;
    }
}

/// Fundo da janela.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Backdrop {
    /// A cor sólida do tema (`SolidBackgroundFillColorBase`).
    #[default]
    Solid,
    /// O Mica do Windows 11: o papel de parede, desfocado e tingido, atrás da janela. Fora do
    /// Windows 11, ou se o sistema recusar, a cor sólida, como no WinUI.
    Mica,
}

/// Janela a abrir.
#[derive(Clone, Debug)]
pub struct WindowOptions {
    /// Título até o `Handler` dar o dele.
    pub title: String,
    /// Largura inicial, em pixels lógicos. Ignorada no Android, que usa a tela toda.
    pub width: u32,
    /// Altura inicial, em pixels lógicos.
    pub height: u32,
    /// Se a tela tem uma `TitleBar`: a janela abre sem a moldura do sistema.
    pub title_bar: bool,
    /// Fundo da janela.
    pub backdrop: Backdrop,
    /// O ícone da janela: o da moldura, da barra de tarefas e do alternador de janelas. `None`
    /// deixa o do sistema.
    pub icon: Option<WindowIcon>,
    /// O nome com que o ambiente agrupa e reconhece a janela: o `app_id` do Wayland e a classe
    /// (`WM_CLASS`) do X11. É o que casa a janela com o atalho `.desktop` do app. `None` deixa o
    /// nome do executável. Não vale no Windows nem no macOS.
    pub app_id: Option<String>,
    /// Arquivo da guarda da abertura: se a abertura anterior caiu antes do primeiro quadro (um
    /// driver de vídeo que derruba o processo), esta desenha de um jeito mais simples, um
    /// degrau por vez, e o degrau que funcionou fica gravado nele. `None` desliga a guarda.
    /// Apagar o arquivo volta ao desenho normal.
    pub render_guard: Option<std::path::PathBuf>,
    #[cfg(target_os = "android")]
    android_app: Option<AndroidApp>,
}

impl WindowOptions {
    /// Janela de `width` × `height` pixels lógicos.
    pub fn new(title: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            title_bar: false,
            backdrop: Backdrop::Solid,
            icon: None,
            app_id: None,
            render_guard: None,
            #[cfg(target_os = "android")]
            android_app: None,
        }
    }

    /// `AndroidApp` recebido em `android_main`. Obrigatório no Android.
    #[cfg(target_os = "android")]
    pub fn android_app(mut self, app: AndroidApp) -> Self {
        self.android_app = Some(app);
        self
    }
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self::new("Tomni UI", 800, 600)
    }
}

/// Falha ao abrir a janela ou ao renderizar.
#[derive(Debug)]
pub enum Error {
    /// O loop de eventos não abriu (sem servidor gráfico, por exemplo).
    EventLoop(winit::error::EventLoopError),
    /// A janela não foi criada.
    Window(winit::error::OsError),
    /// A superfície da janela não foi criada ou o `vello` falhou ao renderizar.
    Render(vello::Error),
    /// O renderizador da GPU não foi criado.
    Gpu(GpuError),
    /// No Android, falta o [`WindowOptions::android_app`].
    #[cfg(target_os = "android")]
    NoAndroidApp,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventLoop(e) => write!(f, "loop de eventos: {e}"),
            Self::Window(e) => write!(f, "janela: {e}"),
            Self::Render(e) => write!(f, "render na GPU: {e}"),
            Self::Gpu(e) => write!(f, "{e}"),
            #[cfg(target_os = "android")]
            Self::NoAndroidApp => write!(f, "WindowOptions::android_app não foi chamado"),
        }
    }
}

impl std::error::Error for Error {}
