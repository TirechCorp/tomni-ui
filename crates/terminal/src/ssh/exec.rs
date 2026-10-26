//! Comando remoto sem terminal: o `exec` do SSH, para o `ssh-copy-id` e para quem precisa só
//! da saída de um comando.

use russh::ChannelMsg;

use super::link::SshLink;
use super::{SshBuilder, ssh_error};
use crate::transport::error_message;

/// O resultado de um comando remoto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    /// O código de saída. `None` se o servidor não avisou (ou o programa morreu por um sinal).
    pub status: Option<u32>,
    /// A saída padrão, em bytes.
    pub stdout: Vec<u8>,
    /// A saída de erro, em bytes.
    pub stderr: Vec<u8>,
}

impl ExecOutput {
    /// Se o comando saiu com o código 0.
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// A saída padrão como texto (bytes inválidos viram `�`).
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// A saída de erro como texto (bytes inválidos viram `�`).
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

impl SshBuilder {
    /// Conecta (com a cadeia de saltos, o proxy, a conferência da chave do servidor e a
    /// autenticação deste builder), roda `command` num canal `exec`, manda `stdin` e fecha a
    /// entrada. Bloqueia até o fim, numa thread própria com o runtime do tokio. Não abre
    /// terminal.
    ///
    /// ```no_run
    /// use tomni_ui_terminal::ssh::SshBuilder;
    ///
    /// let saida = SshBuilder::new("servidor", 22, "ana")
    ///     .password("segredo")
    ///     .exec("uname -s", b"")?;
    /// println!("{}", saida.stdout_text());
    /// # Ok::<(), String>(())
    /// ```
    pub fn exec(&self, command: &str, stdin: &[u8]) -> Result<ExecOutput, String> {
        on_thread(self, |remote| remote.exec(command, stdin))
    }
}

/// Roda `work` numa thread própria com uma conexão aberta em `builder`. A thread é nova para o
/// `block_on` do tokio não brigar com quem chama de dentro de outro runtime.
pub(super) fn on_thread<T: Send>(
    builder: &SshBuilder,
    work: impl FnOnce(&SshLink) -> Result<T, String> + Send,
) -> Result<T, String> {
    let builder = builder.clone();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("ssh-exec".to_owned())
            .spawn_scoped(scope, move || {
                let remote = SshLink::open(&builder)?;
                let result = work(&remote);
                remote.close();
                result
            })
            .map_err(|error| error_message(&error))?
            .join()
            .unwrap_or_else(|_| Err("a conexão terminou de forma inesperada".to_owned()))
    })
}

impl SshLink {
    /// Roda `command`, manda `stdin` e espera o fim.
    pub(super) fn exec(&self, command: &str, stdin: &[u8]) -> Result<ExecOutput, String> {
        let connected = self
            .connected
            .as_ref()
            .ok_or_else(|| "sem conexão".to_owned())?;
        self.runtime.block_on(async {
            let mut channel = connected
                .handle
                .channel_open_session()
                .await
                .map_err(|e| ssh_error(&e))?;
            channel
                .exec(true, command.as_bytes())
                .await
                .map_err(|e| ssh_error(&e))?;
            if !stdin.is_empty() {
                channel.data(stdin).await.map_err(|e| ssh_error(&e))?;
            }
            channel.eof().await.map_err(|e| ssh_error(&e))?;

            let mut output = ExecOutput {
                status: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
            };
            while let Some(message) = channel.wait().await {
                match message {
                    ChannelMsg::Data { data } => output.stdout.extend_from_slice(&data),
                    ChannelMsg::ExtendedData { data, ext: 1 } => {
                        output.stderr.extend_from_slice(&data);
                    }
                    ChannelMsg::ExitStatus { exit_status } => output.status = Some(exit_status),
                    ChannelMsg::Close => break,
                    _ => {}
                }
            }
            Ok(output)
        })
    }
}
