//! Programas remotos sem aba: o `SshBuilder::launch`, para janelas X11 que não precisam de
//! terminal.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use russh::{ChannelMsg, Sig};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use super::forward::{ForwardTable, Live, Tunnels};
use super::x11::X11State;
use super::{
    Command, Connected, Notice, SshBuilder, connect_chain, open_tcp, request_x11, ssh_error, unique,
};
use crate::transport::error_message;

/// Quanto do fim do stderr o [`RemoteApp`] guarda.
const STDERR_TAIL: usize = 4096;

/// Em que pé está um programa lançado por [`SshBuilder::launch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteAppStatus {
    /// Conectando e autenticando.
    Connecting,
    /// O comando foi pedido ao servidor e segue rodando.
    Running,
    /// O programa acabou. `None` quando o servidor não mandou o código (um sinal, a conexão
    /// fechada).
    Exited(Option<u32>),
    /// Não deu para conectar, autenticar ou subir os túneis, ou a conexão caiu; o motivo vem
    /// escrito para a pessoa.
    Failed(String),
}

#[derive(Debug)]
struct Shared {
    status: RemoteAppStatus,
    stderr: Vec<u8>,
}

/// Um programa remoto que roda numa conexão própria, sem terminal: o `virt-manager` aberto por
/// X11, por exemplo. Vem de [`SshBuilder::launch`]; é `Clone` e a thread de trabalho segue
/// depois de soltar todas as cópias (até o programa acabar ou [`close`](Self::close)).
#[derive(Clone, Debug)]
pub struct RemoteApp {
    shared: Arc<Mutex<Shared>>,
    commands: UnboundedSender<Command>,
}

impl RemoteApp {
    /// Em que pé o programa está.
    pub fn status(&self) -> RemoteAppStatus {
        self.lock().status.clone()
    }

    /// Os últimos 4 KiB do stderr do programa, mais os avisos da conexão (o X11 que não achou
    /// servidor, o túnel que não subiu), como texto.
    pub fn stderr_tail(&self) -> String {
        String::from_utf8_lossy(&self.lock().stderr).into_owned()
    }

    /// Fecha a conexão: o programa remoto perde o X11 e a conexão com o servidor acaba. Sem
    /// efeito se ele já acabou.
    pub fn close(&self) {
        let _ = self.commands.send(Command::Close);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Shared {
    fn add_stderr(&mut self, bytes: &[u8]) {
        self.stderr.extend_from_slice(bytes);
        if self.stderr.len() > STDERR_TAIL {
            let cut = self.stderr.len() - STDERR_TAIL;
            self.stderr.drain(..cut);
        }
    }
}

impl SshBuilder {
    /// Roda `command` no servidor numa conexão própria, sem terminal: para programas gráficos,
    /// com [`x11`](Self::x11). Volta na hora; a conexão sobe numa thread, e o [`RemoteApp`]
    /// mostra em que pé está. Vale o que o builder tem: saltos, proxy, autenticação,
    /// [`forward`](Self::forward) (os túneis sobem junto) e [`agent_forward`](Self::agent_forward).
    ///
    /// ```no_run
    /// use tomni_ui_terminal::ssh::{SshBuilder, X11};
    ///
    /// let app = SshBuilder::new("servidor", 22, "ana")
    ///     .agent_auth()
    ///     .x11(X11::trusted())
    ///     .launch("virt-manager --no-fork");
    /// println!("{:?}", app.status());
    /// ```
    pub fn launch(self, command: impl Into<String>) -> RemoteApp {
        let command = command.into();
        let shared = Arc::new(Mutex::new(Shared {
            status: RemoteAppStatus::Connecting,
            stderr: Vec::new(),
        }));
        let (sender, receiver) = mpsc::unbounded_channel();
        let app = RemoteApp {
            shared: Arc::clone(&shared),
            commands: sender.clone(),
        };
        let weak = sender.downgrade();
        let spawned = thread::Builder::new()
            .name("tomni-ui-ssh-app".to_owned())
            .spawn({
                let shared = Arc::clone(&shared);
                move || {
                    // O `sender` fica vivo aqui: soltar todas as cópias do `RemoteApp` não pode
                    // fechar a conexão.
                    let _sender = sender;
                    let result = run(&self, &command, receiver, weak, &shared);
                    let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
                    shared.status = match result {
                        Ok(code) => RemoteAppStatus::Exited(code),
                        Err(message) => RemoteAppStatus::Failed(message),
                    };
                }
            });
        if let Err(error) = spawned {
            app.lock().status = RemoteAppStatus::Failed(error_message(&error));
        }
        app
    }
}

fn run(
    builder: &SshBuilder,
    command: &str,
    receiver: UnboundedReceiver<Command>,
    weak: mpsc::WeakUnboundedSender<Command>,
    shared: &Arc<Mutex<Shared>>,
) -> Result<Option<u32>, String> {
    let notice: Notice = {
        let shared = Arc::clone(shared);
        Arc::new(move |line: &str| {
            let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
            shared.add_stderr(line.as_bytes());
            shared.add_stderr(b"\n");
        })
    };
    let stream = open_tcp(builder, &mut |line| notice(line))?;
    let x11 = builder
        .x11
        .as_ref()
        .and_then(|x11| X11State::prepare(x11, &mut |line| notice(line)));
    let table = ForwardTable::new(&builder.forwards);
    table.set_x11(x11.clone());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error_message(&error))?;
    runtime.block_on(app(
        builder, command, stream, receiver, weak, table, x11, notice, shared,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn app(
    builder: &SshBuilder,
    command: &str,
    stream: std::net::TcpStream,
    mut commands: UnboundedReceiver<Command>,
    weak: mpsc::WeakUnboundedSender<Command>,
    table: Arc<ForwardTable>,
    x11: Option<Arc<X11State>>,
    notice: Notice,
    shared: &Arc<Mutex<Shared>>,
) -> Result<Option<u32>, String> {
    let extras = Arc::new(AtomicUsize::new(0));
    let live = Arc::new(Live {
        x11: x11.clone(),
        table,
        extras: Arc::clone(&extras),
        commands: weak,
        notice: Arc::clone(&notice),
    });
    let Connected { handle, jumps } =
        connect_chain(builder, stream, Some(Arc::clone(&live))).await?;
    let mut tunnels = Tunnels::new(Arc::clone(&handle), live);
    tunnels.start_all(builder.exit_on_forward_failure).await?;

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| ssh_error(&e))?;
    if let Some(x11) = x11.as_deref() {
        request_x11(&mut channel, x11, &notice).await?;
    }
    if builder.agent_forward {
        channel
            .agent_forward(false)
            .await
            .map_err(|e| ssh_error(&e))?;
    }
    channel
        .exec(false, command.as_bytes())
        .await
        .map_err(|e| ssh_error(&e))?;
    shared.lock().unwrap_or_else(PoisonError::into_inner).status = RemoteAppStatus::Running;

    let mut exit = None;
    let mut lost = false;
    let mut program_done = false;
    let mut watch = tokio::time::interval(Duration::from_millis(250));
    watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::ChannelClosed) => {
                    if program_done && extras.load(Ordering::Acquire) == 0 {
                        break;
                    }
                }
                Some(Command::Forward(command)) => tunnels.command(command).await,
                Some(Command::Close) | None => {
                    if !program_done {
                        let _ = channel.close().await;
                    }
                    break;
                }
                // Sem terminal: nada para escrever nem redimensionar, e sem subsistemas.
                Some(Command::OpenSubsystem { reply, .. }) => {
                    let _ = reply.send(Err("o programa remoto não tem terminal".to_owned()));
                }
                Some(Command::OpenExec { reply, .. }) => {
                    let _ = reply.send(Err("o programa remoto não tem terminal".to_owned()));
                }
                Some(Command::OpenDirect { reply, .. }) => {
                    let _ = reply.send(Err("o programa remoto não tem terminal".to_owned()));
                }
                Some(_) => {}
            },
            message = channel.wait(), if !program_done => match message {
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    shared
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .add_stderr(&data);
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => exit = Some(exit_status),
                Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                    let name = match signal_name {
                        Sig::Custom(name) => name,
                        other => format!("{other:?}"),
                    };
                    notice(&format!("O programa remoto terminou com o sinal SIG{name}."));
                }
                Some(ChannelMsg::Close) => {
                    if extras.load(Ordering::Acquire) == 0 {
                        break;
                    }
                    // As janelas X11 e os túneis ainda seguram a conexão.
                    program_done = true;
                }
                None => {
                    lost = true;
                    break;
                }
                // A saída padrão não interessa.
                Some(_) => {}
            },
            _ = watch.tick(), if program_done => {
                if handle.is_closed() {
                    break;
                }
            }
        }
    }
    tunnels.shutdown();
    let handle = if lost {
        match unique(handle).await {
            Ok(mut owned) => {
                if let Ok(Err(error)) =
                    tokio::time::timeout(super::END_REASON_WAIT, &mut owned).await
                {
                    return Err(ssh_error(&error));
                }
                Arc::new(owned)
            }
            Err(shared) => shared,
        }
    } else {
        handle
    };
    Connected { handle, jumps }.disconnect().await;
    Ok(exit)
}
