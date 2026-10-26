//! Uma conexão SSH aberta e sem terminal, para quem precisa de canais próprios (o SFTP do
//! `tomni-ui-files`) e não de um shell.

use std::fmt;
use std::future::Future;
use std::io::{self, Read};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::task::{Context, Poll};
use std::time::Duration;

use russh::client::Msg;
use russh::{Channel, ChannelMsg, ChannelStream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc::{UnboundedSender, WeakUnboundedSender};

use super::exec::ExecOutput;
use super::{Command, Connected, SshBuilder, connect_chain, open_tcp, ssh_error};
use crate::transport::error_message;

/// Uma conexão SSH autenticada (com a cadeia de saltos, o proxy e a conferência da chave do
/// servidor do [`SshBuilder`]), sem shell nem terminal. Tem o seu runtime do tokio, que só
/// avança dentro de [`block_on`](Self::block_on): o canal só anda enquanto alguém espera por
/// ele.
///
/// ```no_run
/// use tomni_ui_terminal::ssh::SshBuilder;
///
/// let link = SshBuilder::new("servidor", 22, "ana")
///     .password("segredo")
///     .connect_link()?;
/// let canal = link.open_subsystem("sftp")?;
/// # drop(canal);
/// link.close();
/// # Ok::<(), String>(())
/// ```
pub struct SshLink {
    pub(super) runtime: Runtime,
    pub(super) connected: Option<Connected>,
}

impl SshBuilder {
    /// Conecta e autentica como [`exec`](Self::exec), mas devolve a conexão aberta, para abrir
    /// canais nela. Bloqueia até autenticar.
    pub fn connect_link(&self) -> Result<SshLink, String> {
        SshLink::open(self)
    }
}

impl SshLink {
    pub(super) fn open(builder: &SshBuilder) -> Result<Self, String> {
        let stream = open_tcp(builder, &mut |_| {})?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error_message(&error))?;
        let connected = runtime.block_on(connect_chain(builder, stream, None))?;
        Ok(Self {
            runtime,
            connected: Some(connected),
        })
    }

    /// Roda `future` no runtime da conexão até acabar. Não chame de dentro de outro runtime
    /// do tokio.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }

    /// Abre um canal com o subsistema `name` (`"sftp"`, por exemplo).
    pub fn open_subsystem(&self, name: &str) -> Result<SshChannel, String> {
        let connected = self
            .connected
            .as_ref()
            .ok_or_else(|| "sem conexão".to_owned())?;
        self.runtime.block_on(async {
            let channel = connected
                .handle
                .channel_open_session()
                .await
                .map_err(|e| ssh_error(&e))?;
            channel
                .request_subsystem(true, name)
                .await
                .map_err(|e| ssh_error(&e))?;
            Ok(SshChannel::new(channel.into_stream()))
        })
    }

    /// Se o servidor ainda mantém a conexão.
    pub fn is_open(&self) -> bool {
        self.connected
            .as_ref()
            .is_some_and(|connected| !connected.handle.is_closed())
    }

    /// Desconecta (o canal aberto deixa de funcionar). Pode ser chamado de novo sem efeito.
    pub fn disconnect(&mut self) {
        if let Some(connected) = self.connected.take() {
            self.runtime.block_on(connected.disconnect());
        }
    }

    /// Desconecta e solta a conexão.
    pub fn close(mut self) {
        self.disconnect();
    }
}

/// Um canal SSH como fluxo de bytes (`AsyncRead` + `AsyncWrite`).
pub struct SshChannel {
    stream: ChannelStream<Msg>,
    /// Nos canais abertos pelo [`SshChannelOpener`]: avisa a conexão do terminal quando o canal
    /// é solto.
    _guard: Option<ChannelGuard>,
}

impl fmt::Debug for SshChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshChannel").finish_non_exhaustive()
    }
}

impl SshChannel {
    pub(super) fn new(stream: ChannelStream<Msg>) -> Self {
        Self {
            stream,
            _guard: None,
        }
    }

    pub(super) fn counted(stream: ChannelStream<Msg>, guard: ChannelGuard) -> Self {
        Self {
            stream,
            _guard: Some(guard),
        }
    }
}

/// Conta um canal extra na conexão do terminal enquanto existir.
pub(super) struct ChannelGuard {
    pub(super) extras: Arc<AtomicUsize>,
    pub(super) commands: WeakUnboundedSender<Command>,
}

impl Drop for ChannelGuard {
    fn drop(&mut self) {
        self.extras.fetch_sub(1, Ordering::AcqRel);
        if let Some(commands) = self.commands.upgrade() {
            let _ = commands.send(Command::ChannelClosed);
        }
    }
}

/// Abre canais de subsistema (o SFTP) na conexão SSH de um terminal, sem outra conexão nem
/// outra autenticação. Vem de [`Session::ssh_channels`](crate::Session::ssh_channels); é
/// `Clone + Send + Sync` e feito para as threads de trabalho.
///
/// Se o shell do terminal sai (`exit`) com canais abertos, a aba mostra o encerramento e a
/// conexão continua até o último canal fechar. Depois de fechar a aba, ou de
/// [`restart`](crate::Session::restart), o opener antigo só devolve erro.
#[derive(Clone, Debug)]
pub struct SshChannelOpener {
    commands: UnboundedSender<Command>,
}

impl SshChannelOpener {
    pub(super) fn new(commands: UnboundedSender<Command>) -> Self {
        Self { commands }
    }

    /// Abre um canal com o subsistema `name` (`"sftp"`). Bloqueia até o servidor aceitar ou
    /// `timeout` passar. Um erro `ConnectionReset` quer dizer que a sessão SSH acabou.
    pub fn open_subsystem(&self, name: &str, timeout: Duration) -> io::Result<SshChannel> {
        let (reply, answer) = mpsc::sync_channel(1);
        let request = Command::OpenSubsystem {
            name: name.to_owned(),
            reply,
        };
        if self.commands.send(request).is_err() {
            return Err(session_closed());
        }
        match answer.recv_timeout(timeout) {
            Ok(Ok(channel)) => Ok(channel),
            Ok(Err(message)) => Err(io::Error::other(message)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "o servidor não respondeu ao pedido do canal",
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(session_closed()),
        }
    }
}

impl SshChannelOpener {
    /// Abre um canal `direct-tcpip` até `host:port`, visto do servidor, na conexão do terminal,
    /// sem escutar porta nenhuma aqui. Bloqueia até o servidor aceitar ou `timeout` passar. Só
    /// o servidor de destino abre (os saltos recusam, como nos túneis). Um erro
    /// `ConnectionReset` quer dizer que a sessão SSH acabou; a recusa do servidor vem escrita
    /// para a pessoa ("o servidor proíbe túneis (AllowTcpForwarding)", "Nada escuta em
    /// db:5432 no servidor.").
    ///
    /// O canal conta como canal extra: a conexão continua enquanto ele existir, mesmo que o
    /// shell do terminal saia.
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use tomni_ui_terminal::Session;
    ///
    /// # fn exemplo(sessao: Session) -> std::io::Result<()> {
    /// let canais = sessao.ssh_channels().expect("a sessão é SSH");
    /// let canal = canais.open_direct("127.0.0.1", 3306, Duration::from_secs(5))?;
    /// # drop(canal);
    /// # Ok(()) }
    /// ```
    pub fn open_direct(&self, host: &str, port: u16, timeout: Duration) -> io::Result<SshChannel> {
        let (reply, answer) = mpsc::sync_channel(1);
        let request = Command::OpenDirect {
            host: host.to_owned(),
            port,
            reply,
        };
        if self.commands.send(request).is_err() {
            return Err(session_closed());
        }
        match answer.recv_timeout(timeout) {
            Ok(Ok(channel)) => Ok(channel),
            Ok(Err(message)) => Err(io::Error::other(message)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "o servidor não respondeu ao pedido do canal",
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(session_closed()),
        }
    }
}

/// Um canal `exec` aberto na conexão do terminal, a caminho de quem vai falar com ele.
pub(super) struct ExecChannel {
    channel: Channel<Msg>,
    _guard: ChannelGuard,
}

impl ExecChannel {
    pub(super) fn new(channel: Channel<Msg>, guard: ChannelGuard) -> Self {
        Self {
            channel,
            _guard: guard,
        }
    }
}

impl fmt::Debug for ExecChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecChannel").finish_non_exhaustive()
    }
}

/// Quanto de `stdin` vai em cada mensagem de dados do `exec`.
const EXEC_BLOCK: usize = 32 * 1024;

impl SshChannelOpener {
    /// Roda `command` num canal `exec` novo desta conexão, manda `stdin` (lido até o fim, em
    /// blocos de 32 KiB), fecha a entrada e espera o código de saída. `sent` é chamado com o
    /// total enviado a cada bloco. Bloqueia; chame de uma thread. O `timeout` vale para o
    /// servidor aceitar o canal, não para o comando inteiro.
    ///
    /// Se `stdin` devolver erro (é assim que se cancela um envio longo), o canal é fechado
    /// **sem** o fim de entrada, e esse erro é devolvido: o comando do servidor vê a conexão
    /// do canal cair, e não um arquivo que acabou. Se o comando sai antes de ler tudo, o que
    /// ele escreveu e o código de saída voltam em [`ExecOutput`], e só a falta de código vira
    /// erro.
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use tomni_ui_terminal::Session;
    ///
    /// # fn exemplo(sessao: Session) -> std::io::Result<()> {
    /// let canais = sessao.ssh_channels().expect("a sessão é SSH");
    /// let saida = canais.exec(
    ///     "cat > /tmp/dados",
    ///     &mut &b"dados"[..],
    ///     &mut |_enviado| {},
    ///     Duration::from_secs(20),
    /// )?;
    /// assert!(saida.success());
    /// # Ok(()) }
    /// ```
    pub fn exec(
        &self,
        command: &str,
        stdin: &mut dyn Read,
        sent: &mut dyn FnMut(u64),
        timeout: Duration,
    ) -> io::Result<ExecOutput> {
        self.exec_with(command, stdin, sent, timeout, None)
    }

    /// Como [`exec`](Self::exec), mas para quando `cancel` vira `true`: o canal é fechado (o
    /// comando do servidor perde a conexão e recebe `SIGHUP`, se for o caso) e o erro devolvido
    /// é `Interrupted`. A bandeira é olhada a cada bloco de `stdin` e a cada 100 ms de espera
    /// pela saída, então não pesa num comando que responde.
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use std::sync::atomic::AtomicBool;
    /// use std::time::Duration;
    /// use tomni_ui_terminal::Session;
    ///
    /// # fn exemplo(sessao: Session) -> std::io::Result<()> {
    /// let canais = sessao.ssh_channels().expect("a sessão é SSH");
    /// let parar = Arc::new(AtomicBool::new(false));
    /// let saida = canais.exec_cancelable(
    ///     "sleep 600",
    ///     &mut std::io::empty(),
    ///     &mut |_| {},
    ///     Duration::from_secs(20),
    ///     &parar,
    /// );
    /// // Outra thread faz `parar.store(true, Ordering::Relaxed)`; aqui volta `Interrupted`.
    /// # drop(saida); Ok(()) }
    /// ```
    pub fn exec_cancelable(
        &self,
        command: &str,
        stdin: &mut dyn Read,
        sent: &mut dyn FnMut(u64),
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> io::Result<ExecOutput> {
        self.exec_with(command, stdin, sent, timeout, Some(cancel))
    }

    fn exec_with(
        &self,
        command: &str,
        stdin: &mut dyn Read,
        sent: &mut dyn FnMut(u64),
        timeout: Duration,
        cancel: Option<&AtomicBool>,
    ) -> io::Result<ExecOutput> {
        let cancelled = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
        let (reply, answer) = mpsc::sync_channel(1);
        let request = Command::OpenExec {
            command: command.to_owned(),
            reply,
        };
        if self.commands.send(request).is_err() {
            return Err(session_closed());
        }
        let opened = match answer.recv_timeout(timeout) {
            Ok(Ok(opened)) => opened,
            Ok(Err(message)) => return Err(io::Error::other(message)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "o servidor não respondeu ao pedido do canal",
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(session_closed()),
        };
        let ExecChannel {
            mut channel,
            _guard,
        } = opened;
        let runtime = Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(async move {
            let mut block = vec![0u8; EXEC_BLOCK];
            let mut total = 0u64;
            let mut write_failed = None;
            loop {
                if cancelled() {
                    let _ = channel.close().await;
                    return Err(cancel_error());
                }
                let read = match stdin.read(&mut block) {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = channel.close().await;
                        return Err(error);
                    }
                };
                if let Err(error) = channel.data(&block[..read]).await {
                    write_failed = Some(ssh_error(&error));
                    break;
                }
                total += read as u64;
                sent(total);
            }
            if write_failed.is_none()
                && let Err(error) = channel.eof().await
            {
                write_failed = Some(ssh_error(&error));
            }
            let mut output = ExecOutput {
                status: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
            };
            loop {
                let message = match tokio::time::timeout(CANCEL_POLL, channel.wait()).await {
                    Ok(Some(message)) => message,
                    Ok(None) => break,
                    Err(_) => {
                        if cancelled() {
                            let _ = channel.close().await;
                            return Err(cancel_error());
                        }
                        continue;
                    }
                };
                match message {
                    ChannelMsg::Data { data } => output.stdout.extend_from_slice(&data),
                    ChannelMsg::ExtendedData { data, ext: 1 } => {
                        output.stderr.extend_from_slice(&data);
                    }
                    ChannelMsg::ExitStatus { exit_status } => output.status = Some(exit_status),
                    ChannelMsg::Failure => {
                        let _ = channel.close().await;
                        return Err(io::Error::other("o servidor recusou executar o comando"));
                    }
                    ChannelMsg::Close => break,
                    _ => {}
                }
            }
            match write_failed {
                Some(message) if output.status.is_none() => Err(io::Error::other(message)),
                _ => Ok(output),
            }
        })
    }
}

/// De quanto em quanto tempo o `exec` olha a bandeira de cancelamento enquanto espera a saída.
const CANCEL_POLL: Duration = Duration::from_millis(100);

fn cancel_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "o comando foi cancelado")
}

fn session_closed() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionReset, "a sessão SSH foi fechada")
}

impl AsyncRead for SshChannel {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for SshChannel {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
