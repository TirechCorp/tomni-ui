//! Widgets nativos: um controle do sistema (o `WebView`) posto como janela filha da janela do
//! app, por cima do que o `Ui` pinta.
//!
//! A plataforma instala a janela ([`set_host`]) e gira, a cada volta do loop, o que os
//! widgets nativos registram ([`add_pump`]: o GTK do WebKitGTK, no Linux). O `Ui` diz a cada
//! frame onde o widget aparece e se alguma coisa o cobre ([`Widget::place_native`]); coberto
//! (um `ContentDialog`, um `Flyout`), o widget esconde a janela filha e pinta uma captura no
//! lugar, porque nada que o `Ui` pinta fica por cima de uma janela do sistema. Sem janela (nos
//! testes, no lint, nas ferramentas), os widgets nativos usam um motor falso.
//!
//! [`Widget::place_native`]: crate::Widget::place_native

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kurbo::Rect;
use raw_window_handle::{HandleError, HasWindowHandle, WindowHandle};

/// Intervalo em que a plataforma gira os [`add_pump`] enquanto algum está registrado: um
/// frame a 60 Hz.
pub const PUMP_INTERVAL: Duration = Duration::from_millis(16);

/// Onde um widget nativo aparece, dito pelo `Ui` a cada frame em que muda.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativePlace {
    /// Retângulo na janela, em pixels lógicos; `None` se o widget não aparece (oculto, fora
    /// da área visível de um `ScrollViewer`, sem tamanho).
    pub rect: Option<Rect>,
    /// Se alguma coisa pintada pelo `Ui` fica por cima de parte dele (uma camada aberta, um
    /// irmão `raised`) ou se um `ScrollViewer` corta uma parte dele: a janela filha precisa
    /// sumir, e o widget pinta uma captura no lugar.
    pub covered: bool,
    /// Se o tema é o escuro: a página segue (`prefers-color-scheme`).
    pub dark: bool,
}

impl NativePlace {
    /// Fora da tela.
    pub const HIDDEN: Self = Self {
        rect: None,
        covered: false,
        dark: false,
    };
}

/// Pedido de foco de um widget nativo ao `Ui` ([`Widget::take_native_focus`]).
///
/// [`Widget::take_native_focus`]: crate::Widget::take_native_focus
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeFocus {
    /// O controle nativo ganhou o foco sozinho (um clique nele): a árvore passa o foco para o
    /// widget.
    Took,
    /// O foco sai para a próxima parada de Tab (o F6 dentro da página).
    Next,
    /// O foco sai para a parada de Tab anterior (o Shift+F6).
    Prev,
}

thread_local! {
    static HOST: RefCell<Option<Rc<dyn HasWindowHandle>>> = const { RefCell::new(None) };
    static PUMPS: RefCell<Vec<Pump>> = const { RefCell::new(Vec::new()) };
    static EXPECTED: Cell<bool> = const { Cell::new(false) };
}

static X11: AtomicBool = AtomicBool::new(false);

struct Pump {
    name: &'static str,
    turn: Box<dyn FnMut() -> bool>,
}

/// Instala a janela do app, onde os widgets nativos põem as janelas filhas. A plataforma
/// instala ao abrir a janela, na thread da interface.
pub fn set_host(window: Rc<dyn HasWindowHandle>) {
    HOST.with(|host| *host.borrow_mut() = Some(window));
}

/// Tira a janela que [`set_host`] instalou.
pub fn clear_host() {
    HOST.with(|host| host.borrow_mut().take());
    EXPECTED.set(false);
}

/// Avisa que uma janela vai abrir nesta thread ([`set_host`]): o app monta a tela (e roda o
/// primeiro frame) antes de a janela existir, e os widgets nativos esperam por ela em vez de
/// cair no motor falso.
pub fn expect_host() {
    EXPECTED.set(true);
}

/// Se a janela foi anunciada ([`expect_host`]) e ainda não chegou.
pub fn host_pending() -> bool {
    EXPECTED.get() && HOST.with(|host| host.borrow().is_none())
}

/// A janela do app, se a plataforma abriu uma nesta thread.
pub fn host() -> Option<Host> {
    HOST.with(|host| host.borrow().clone()).map(Host)
}

/// A janela do app ([`host`]), para criar uma janela filha.
#[derive(Clone)]
pub struct Host(Rc<dyn HasWindowHandle>);

impl HasWindowHandle for Host {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host").finish_non_exhaustive()
    }
}

/// Registra `turn`, que a plataforma chama a cada volta do loop (a cada [`PUMP_INTERVAL`],
/// no máximo, enquanto houver algum): o motor do controle nativo processa os eventos dele.
/// `turn` devolve se ainda precisa girar; ao devolver `false`, sai da lista. Um segundo
/// registro com o mesmo `name` é ignorado.
pub fn add_pump(name: &'static str, turn: impl FnMut() -> bool + 'static) {
    PUMPS.with(|pumps| {
        let mut pumps = pumps.borrow_mut();
        if pumps.iter().all(|pump| pump.name != name) {
            pumps.push(Pump {
                name,
                turn: Box::new(turn),
            });
        }
    });
}

/// Gira os [`add_pump`] registrados. Devolve em quanto tempo girar de novo, ou `None` se não
/// sobrou nenhum.
pub fn pump() -> Option<Duration> {
    let mut pumps = PUMPS.with(|pumps| std::mem::take(&mut *pumps.borrow_mut()));
    pumps.retain_mut(|pump| (pump.turn)());
    PUMPS.with(|registered| {
        // Um `turn` pode ter registrado outro.
        let mut registered = registered.borrow_mut();
        pumps.append(&mut registered);
        *registered = pumps;
        (!registered.is_empty()).then_some(PUMP_INTERVAL)
    })
}

/// Pede à plataforma uma janela X11 (pelo XWayland numa sessão Wayland): o WebKitGTK só põe
/// uma janela filha numa janela X11. Chamar antes de a janela abrir (ao montar a tela).
pub fn require_x11() {
    X11.store(true, Ordering::Relaxed);
}

/// Se algum widget pediu [`require_x11`].
pub fn x11_required() -> bool {
    X11.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn pump_gira_ate_o_registro_pedir_para_sair() {
        let turns = Rc::new(Cell::new(0));
        let counted = Rc::clone(&turns);
        add_pump("teste", move || {
            counted.set(counted.get() + 1);
            counted.get() < 2
        });
        add_pump("teste", || panic!("registro repetido"));
        assert_eq!(pump(), Some(PUMP_INTERVAL));
        assert_eq!(pump(), None);
        assert_eq!(pump(), None);
        assert_eq!(turns.get(), 2);
    }

    #[test]
    fn sem_janela_nao_ha_host() {
        assert!(host().is_none());
    }

    #[test]
    fn janela_anunciada_fica_pendente_ate_chegar() {
        struct Janela;
        impl HasWindowHandle for Janela {
            fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
                Err(HandleError::Unavailable)
            }
        }
        assert!(!host_pending());
        expect_host();
        assert!(host_pending());
        set_host(Rc::new(Janela));
        assert!(!host_pending());
        clear_host();
        assert!(!host_pending());
        assert!(host().is_none());
    }
}
