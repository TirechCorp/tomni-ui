//! Acordar o loop de eventos de fora da thread da interface: uma thread que lê da rede ou de um
//! processo chama [`wake`] quando chegam dados, e a plataforma roda o próximo frame, que aplica
//! os signals e efeitos que a thread mudou mesmo sem widget animando.

use std::sync::{Arc, Mutex, PoisonError};

type Waker = Arc<dyn Fn() + Send + Sync>;

static WAKER: Mutex<Option<Waker>> = Mutex::new(None);

/// Instala o que [`wake`] chama. A plataforma instala ao abrir a janela; sem janela (nos
/// testes), [`wake`] não faz nada.
pub fn set_waker(waker: impl Fn() + Send + Sync + 'static) {
    *WAKER.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(waker));
}

/// Tira o que [`set_waker`] instalou.
pub fn clear_waker() {
    *WAKER.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

/// Acorda o loop de eventos, de qualquer thread: os widgets que animam ([`Widget::ticking`])
/// são consultados de novo e o frame roda. Barato; chamar a cada bloco de dados que chega.
///
/// [`Widget::ticking`]: crate::Widget::ticking
pub fn wake() {
    let waker = WAKER.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(waker) = waker {
        waker();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn chama_o_waker_instalado_de_outra_thread() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        wake();
        set_waker(|| {
            CALLS.fetch_add(1, Ordering::SeqCst);
        });
        std::thread::spawn(wake).join().unwrap();
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        clear_waker();
        wake();
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }
}
