//! As notificações do sistema: o balão do Windows (a notificação do sistema, que fica na
//! Central de Notificações), a notificação do macOS e a do Linux (`org.freedesktop.Notifications`).
//!
//! ```ignore
//! let id = Notification::new("Build terminou")
//!     .body("Nenhum erro")
//!     .tag("build")
//!     .on_click(move || janela.focus())
//!     .show();
//! // Mais tarde, se o usuário já viu o resultado por outro caminho:
//! notification::close(id);
//! ```
//!
//! O clique na notificação roda a ação de [`Notification::on_click`] na thread da interface, no
//! frame seguinte, como os cliques na bandeja. Uma notificação com o mesmo [`Notification::tag`]
//! de uma anterior toma o lugar dela no sistema, e o clique na antiga deixa de valer.
//!
//! Quem mostra é um [`NotificationBackend`]: a plataforma instala o de verdade ao abrir a janela
//! principal; o `TestApp`, um falso que guarda o que foi mostrado. Sem backend (nas ferramentas
//! e no `lint`), [`Notification::show`] não mostra nada. O estado é por thread, então testes em
//! paralelo não se misturam.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc;

use crate::wake::wake;

/// Quantas notificações guardam a ação do clique. A mais antiga além disso fica no sistema,
/// mas o clique nela não faz nada.
const KEPT: usize = 64;

/// A notificação como o sistema a recebe.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationState {
    /// A linha em destaque.
    pub title: String,
    /// O texto de baixo; vazio quando não há.
    pub body: String,
    /// O grupo que se substitui: a notificação nova toma o lugar da anterior com o mesmo.
    pub tag: Option<String>,
}

/// Quem mostra as notificações. Roda na thread da interface; o clique volta por
/// [`NotificationSender`].
pub trait NotificationBackend {
    /// Mostra a notificação `id`. Se `state.tag` é o de uma anterior, ela é substituída.
    fn show(&self, id: u64, state: &NotificationState);
    /// Tira a notificação `id`, se ainda estiver à vista.
    fn close(&self, id: u64);
}

/// Por onde o backend avisa o clique, de qualquer thread. Cada aviso acorda o laço de eventos.
#[derive(Clone)]
pub struct NotificationSender {
    tx: mpsc::Sender<u64>,
}

impl NotificationSender {
    /// O usuário clicou na notificação `id`.
    pub fn clicked(&self, id: u64) {
        let _ = self.tx.send(id);
        wake();
    }
}

struct Shown {
    id: u64,
    tag: Option<String>,
    on_click: Option<Rc<dyn Fn()>>,
}

struct Registry {
    backend: Option<Rc<dyn NotificationBackend>>,
    app_name: Option<String>,
    shown: VecDeque<Shown>,
    next: u64,
    tx: mpsc::Sender<u64>,
    rx: mpsc::Receiver<u64>,
    ready: VecDeque<u64>,
}

impl Registry {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            backend: None,
            app_name: None,
            shown: VecDeque::new(),
            next: 1,
            tx,
            rx,
            ready: VecDeque::new(),
        }
    }

    fn collect(&mut self) {
        while let Ok(id) = self.rx.try_recv() {
            self.ready.push_back(id);
        }
    }
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::new());
}

/// Instala quem mostra as notificações desta thread. A plataforma chama ao abrir a janela
/// principal; o `TestApp`, com um falso.
#[doc(hidden)]
pub fn set_backend(backend: Rc<dyn NotificationBackend>) {
    REGISTRY.with(|r| r.borrow_mut().backend = Some(backend));
}

/// Tira o backend (a janela fechou). As notificações seguintes não aparecem.
#[doc(hidden)]
pub fn clear_backend() {
    REGISTRY.with(|r| r.borrow_mut().backend = None);
}

/// O nome do app no cabeçalho das notificações (no Windows e no Linux; no macOS é o do
/// pacote). Chame antes de abrir a janela: a plataforma o lê ao instalar o backend. Sem ele, vale
/// o título inicial da janela principal.
pub fn set_app_name(name: impl Into<String>) {
    REGISTRY.with(|r| r.borrow_mut().app_name = Some(name.into()));
}

/// O nome dado em [`set_app_name`].
#[doc(hidden)]
pub fn app_name() -> Option<String> {
    REGISTRY.with(|r| r.borrow().app_name.clone())
}

/// Se há quem mostre as notificações agora.
pub fn available() -> bool {
    REGISTRY.with(|r| r.borrow().backend.is_some())
}

/// Por onde o backend avisa os cliques.
#[doc(hidden)]
pub fn sender() -> NotificationSender {
    NotificationSender {
        tx: REGISTRY.with(|r| r.borrow().tx.clone()),
    }
}

/// Volta ao começo: sem backend, sem nome, sem notificações e sem cliques. O `TestApp` chama
/// ao nascer.
#[doc(hidden)]
pub fn reset() {
    REGISTRY.with(|r| *r.borrow_mut() = Registry::new());
}

/// Tira a notificação `id` da vista do sistema e esquece a ação dela. Não faz nada se ela já
/// saiu, foi substituída ou nunca existiu.
pub fn close(id: u64) {
    let backend = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let before = r.shown.len();
        r.shown.retain(|s| s.id != id);
        (r.shown.len() != before)
            .then(|| r.backend.clone())
            .flatten()
    });
    if let Some(backend) = backend {
        backend.close(id);
    }
}

/// Roda as ações dos cliques que chegaram. O `Ui::frame` chama no começo. Retorna se alguma
/// rodou.
#[doc(hidden)]
pub fn deliver() -> bool {
    let ready = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.collect();
        std::mem::take(&mut r.ready)
    });
    let mut any = false;
    for id in ready {
        let action = REGISTRY.with(|r| {
            let mut r = r.borrow_mut();
            let at = r.shown.iter().position(|s| s.id == id)?;
            r.shown.remove(at)?.on_click
        });
        if let Some(action) = action {
            action();
            any = true;
        }
    }
    any
}

/// Se há clique esperando para o próximo [`deliver`]. O `Ui::deadline` usa para a plataforma
/// rodar o frame.
#[doc(hidden)]
pub fn has_event() -> bool {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.collect();
        !r.ready.is_empty()
    })
}

/// Uma notificação em construção; termina em [`Notification::show`].
pub struct Notification {
    state: NotificationState,
    on_click: Option<Rc<dyn Fn()>>,
}

impl Notification {
    /// Uma notificação com a linha em destaque `title`.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            state: NotificationState {
                title: title.into(),
                ..NotificationState::default()
            },
            on_click: None,
        }
    }

    /// O texto de baixo.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.state.body = body.into();
        self
    }

    /// O grupo: a notificação toma o lugar da anterior com o mesmo `tag`, em vez de empilhar.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.state.tag = Some(tag.into());
        self
    }

    /// O que fazer quando o usuário clica na notificação. Roda na thread da interface.
    pub fn on_click(mut self, f: impl Fn() + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    /// Mostra a notificação e devolve o número dela, o de [`close`]. Sem backend não mostra
    /// nada, e o número não vale para nada.
    pub fn show(self) -> u64 {
        let (id, backend) = REGISTRY.with(|r| {
            let mut r = r.borrow_mut();
            let id = r.next;
            r.next += 1;
            if let Some(tag) = &self.state.tag {
                r.shown.retain(|s| s.tag.as_ref() != Some(tag));
            }
            if r.backend.is_some() {
                r.shown.push_back(Shown {
                    id,
                    tag: self.state.tag.clone(),
                    on_click: self.on_click,
                });
                while r.shown.len() > KEPT {
                    r.shown.pop_front();
                }
            }
            (id, r.backend.clone())
        });
        if let Some(backend) = backend {
            backend.show(id, &self.state);
        }
        id
    }
}

impl std::fmt::Debug for Notification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notification")
            .field("state", &self.state)
            .field("on_click", &self.on_click.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct Falso {
        vistas: RefCell<Vec<(u64, NotificationState)>>,
        fechadas: RefCell<Vec<u64>>,
    }

    impl NotificationBackend for Falso {
        fn show(&self, id: u64, state: &NotificationState) {
            self.vistas.borrow_mut().push((id, state.clone()));
        }
        fn close(&self, id: u64) {
            self.fechadas.borrow_mut().push(id);
        }
    }

    fn preparar() -> Rc<Falso> {
        reset();
        let falso = Rc::new(Falso::default());
        set_backend(falso.clone());
        falso
    }

    #[test]
    fn mostra_o_titulo_o_texto_e_o_grupo() {
        let falso = preparar();
        let id = Notification::new("Pronto")
            .body("Terminou")
            .tag("aba")
            .show();
        let vistas = falso.vistas.borrow();
        assert_eq!(vistas.len(), 1);
        assert_eq!(vistas[0].0, id);
        assert_eq!(vistas[0].1.title, "Pronto");
        assert_eq!(vistas[0].1.body, "Terminou");
        assert_eq!(vistas[0].1.tag.as_deref(), Some("aba"));
    }

    #[test]
    fn o_clique_roda_a_acao_no_deliver_e_uma_vez_so() {
        let _falso = preparar();
        let cliques = Rc::new(Cell::new(0));
        let id = Notification::new("A")
            .on_click({
                let cliques = cliques.clone();
                move || cliques.set(cliques.get() + 1)
            })
            .show();
        sender().clicked(id);
        assert!(has_event());
        assert_eq!(cliques.get(), 0, "nunca síncrono");
        assert!(deliver());
        assert_eq!(cliques.get(), 1);
        sender().clicked(id);
        assert!(!deliver(), "a notificação sai do sistema ao ser clicada");
        assert_eq!(cliques.get(), 1);
    }

    #[test]
    fn a_do_mesmo_grupo_substitui_e_o_clique_na_antiga_nao_vale() {
        let _falso = preparar();
        let qual = Rc::new(Cell::new(0));
        let antiga = Notification::new("1")
            .tag("aba")
            .on_click({
                let qual = qual.clone();
                move || qual.set(1)
            })
            .show();
        let nova = Notification::new("2")
            .tag("aba")
            .on_click({
                let qual = qual.clone();
                move || qual.set(2)
            })
            .show();
        sender().clicked(antiga);
        assert!(!deliver());
        sender().clicked(nova);
        assert!(deliver());
        assert_eq!(qual.get(), 2);
    }

    #[test]
    fn fechar_tira_do_sistema_e_esquece_o_clique() {
        let falso = preparar();
        let cliques = Rc::new(Cell::new(0));
        let id = Notification::new("A")
            .on_click({
                let cliques = cliques.clone();
                move || cliques.set(cliques.get() + 1)
            })
            .show();
        close(id);
        close(id);
        assert_eq!(*falso.fechadas.borrow(), [id], "fecha uma vez só");
        sender().clicked(id);
        assert!(!deliver());
        assert_eq!(cliques.get(), 0);
    }

    #[test]
    fn sem_backend_nao_mostra_nem_guarda() {
        reset();
        assert!(!available());
        let id = Notification::new("A")
            .on_click(|| panic!("não roda"))
            .show();
        sender().clicked(id);
        assert!(!deliver());
    }

    #[test]
    fn guarda_so_as_ultimas_acoes() {
        let _falso = preparar();
        let cliques = Rc::new(Cell::new(0));
        let primeira = Notification::new("0")
            .on_click({
                let cliques = cliques.clone();
                move || cliques.set(cliques.get() + 1)
            })
            .show();
        for i in 0..KEPT {
            let _ = Notification::new(i.to_string()).show();
        }
        sender().clicked(primeira);
        assert!(!deliver());
        assert_eq!(cliques.get(), 0);
    }

    #[test]
    fn o_clique_de_outra_thread_acorda_e_chega() {
        let _falso = preparar();
        let cliques = Rc::new(Cell::new(0));
        let id = Notification::new("A")
            .on_click({
                let cliques = cliques.clone();
                move || cliques.set(cliques.get() + 1)
            })
            .show();
        let emissor = sender();
        std::thread::spawn(move || emissor.clicked(id))
            .join()
            .unwrap();
        assert!(has_event());
        assert!(deliver());
        assert_eq!(cliques.get(), 1);
    }
}
