//! A conexão TCP vista de fora da pilha: [`TcpStream`], que fala com o laço da pilha por um
//! estado compartilhado (dois buffers de preparo, os avisos e o erro) em vez de um canal por
//! trecho de dados.

use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Notify;

/// O que a [`TcpStream`] e o laço da pilha dividem.
pub(crate) struct Shared {
    inner: Mutex<Inner>,
    /// Acorda o laço da pilha.
    wake: Arc<Notify>,
    /// Quanto cabe em cada buffer de preparo (o `tcp_buffer` da pilha).
    capacity: usize,
    pub(crate) peer: SocketAddr,
}

#[derive(Default)]
pub(crate) struct Inner {
    /// Dados recebidos do servidor, esperando a leitura.
    pub(crate) rx: VecDeque<u8>,
    /// O servidor fechou o lado dele (FIN) e tudo o que mandou já foi para `rx`.
    pub(crate) rx_eof: bool,
    pub(crate) rx_waker: Option<Waker>,
    /// Dados escritos, esperando ir para o socket.
    pub(crate) tx: VecDeque<u8>,
    /// O app pediu o fim da escrita (`shutdown`).
    pub(crate) write_shutdown: bool,
    /// O laço já mandou o FIN.
    pub(crate) write_closed: bool,
    pub(crate) tx_waker: Option<Waker>,
    /// O que deu errado na conexão (RST, pilha parada…).
    pub(crate) error: Option<(io::ErrorKind, String)>,
    /// A `TcpStream` foi largada: o laço fecha a conexão e a recolhe.
    pub(crate) dropped: bool,
    /// O laço terminou com esta conexão.
    pub(crate) closed: bool,
}

impl Shared {
    pub(crate) fn new(peer: SocketAddr, capacity: usize, wake: Arc<Notify>) -> Arc<Shared> {
        Arc::new(Shared {
            inner: Mutex::new(Inner::default()),
            wake,
            capacity,
            peer,
        })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Inner {
    pub(crate) fn wake_reader(&mut self) {
        if let Some(waker) = self.rx_waker.take() {
            waker.wake();
        }
    }

    pub(crate) fn wake_writer(&mut self) {
        if let Some(waker) = self.tx_waker.take() {
            waker.wake();
        }
    }

    pub(crate) fn fail(&mut self, kind: io::ErrorKind, message: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some((kind, message.into()));
        }
        self.closed = true;
        self.wake_reader();
        self.wake_writer();
    }
}

/// Uma conexão TCP da pilha: lê e escreve como qualquer fluxo do `tokio`
/// (`AsyncRead + AsyncWrite + Unpin + Send`).
///
/// Largar a `TcpStream` fecha a conexão (FIN, depois de mandar o que faltava).
pub struct TcpStream {
    shared: Arc<Shared>,
}

impl TcpStream {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }

    /// O endereço do servidor.
    pub fn peer_addr(&self) -> SocketAddr {
        self.shared.peer
    }
}

impl std::fmt::Debug for TcpStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TcpStream")
            .field("peer", &self.shared.peer)
            .finish()
    }
}

fn error_of(inner: &Inner) -> Option<io::Error> {
    inner
        .error
        .as_ref()
        .map(|(kind, message)| io::Error::new(*kind, message.clone()))
}

impl AsyncRead for TcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut inner = self.shared.lock();
        if !inner.rx.is_empty() {
            let n = inner.rx.len().min(buf.remaining());
            let (first, second) = inner.rx.as_slices();
            let from_first = n.min(first.len());
            buf.put_slice(&first[..from_first]);
            buf.put_slice(&second[..n - from_first]);
            inner.rx.drain(..n);
            drop(inner);
            // Abriu espaço: o laço pode trazer mais do socket (e reabrir a janela TCP).
            self.shared.wake.notify_one();
            return Poll::Ready(Ok(()));
        }
        if let Some(error) = error_of(&inner) {
            return Poll::Ready(Err(error));
        }
        if inner.rx_eof || inner.closed {
            return Poll::Ready(Ok(()));
        }
        inner.rx_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut inner = self.shared.lock();
        if let Some(error) = error_of(&inner) {
            return Poll::Ready(Err(error));
        }
        if inner.write_shutdown || inner.closed {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "a conexão foi encerrada",
            )));
        }
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let room = self.shared.capacity.saturating_sub(inner.tx.len());
        if room == 0 {
            inner.tx_waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let n = room.min(buf.len());
        inner.tx.extend(&buf[..n]);
        drop(inner);
        self.shared.wake.notify_one();
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut inner = self.shared.lock();
        if let Some(error) = error_of(&inner) {
            return Poll::Ready(Err(error));
        }
        if inner.tx.is_empty() || inner.closed {
            return Poll::Ready(Ok(()));
        }
        inner.tx_waker = Some(cx.waker().clone());
        Poll::Pending
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut inner = self.shared.lock();
        if !inner.write_shutdown {
            inner.write_shutdown = true;
            self.shared.wake.notify_one();
        }
        if inner.write_closed || inner.closed {
            return Poll::Ready(Ok(()));
        }
        inner.tx_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        self.shared.lock().dropped = true;
        self.shared.wake.notify_one();
    }
}
