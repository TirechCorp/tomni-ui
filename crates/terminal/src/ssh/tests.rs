//! Os testes do `Ssh`, com servidores `russh` locais em `127.0.0.1:0`.

use super::*;
use russh::keys::agent::client::AgentClient;
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

/// O que o servidor de teste recebeu, em ordem.
type Log = Arc<Mutex<Vec<String>>>;

/// Um servidor SSH: aceita a senha "senha" e a chave `client` para o usuário "ana", dá as
/// boas-vindas no shell, ecoa o que chega e sai com 3 no "exit 3". Aceita vários clientes, abre
/// túneis `direct-tcpip` (para os saltos) e, se o cliente encaminha o agente, pergunta a ele
/// quantas chaves tem.
struct Servidor {
    client: PublicKey,
    log: Log,
    /// Os canais com shell: só neles o servidor ecoa (um túnel não é eco).
    shells: Vec<ChannelId>,
    modo: Modo,
    /// Os canais com um comando (`exec`): o comando e o que chegou pelo `stdin` até o `eof`.
    comandos: std::collections::HashMap<ChannelId, (String, Vec<u8>)>,
    /// Se algum canal pediu pseudoterminal (um `exec` com pty é o `command` do cliente).
    pty: bool,
    /// O cookie falso que o cliente mandou no `x11-req`.
    x11_cookie: Option<Vec<u8>>,
    /// As portas que o servidor escuta a pedido do cliente (`-R`), com a tarefa de cada uma.
    remotas: std::collections::HashMap<u32, tokio::task::JoinHandle<()>>,
}

/// Como o servidor responde aos comandos (`exec`).
#[derive(Clone)]
struct Modo {
    /// Um servidor Unix roda o comando num `sh` de verdade, com `HOME` em `home`. Um servidor
    /// Windows simulado não tem `uname`, responde ao PowerShell e guarda o `authorized_keys`
    /// em `arquivo`.
    windows: bool,
    /// No Windows simulado: o usuário é administrador.
    admin: bool,
    home: PathBuf,
    arquivo: Arc<Mutex<String>>,
    /// O servidor proíbe túneis (o `AllowTcpForwarding no`): recusa o `direct-tcpip` e o `-R`.
    bloqueia_tuneis: Arc<AtomicBool>,
    /// O servidor responde "não" ao pedido de X11.
    recusa_x11: Arc<AtomicBool>,
    /// O servidor abre um canal X11 para o cliente assim que a autenticação passa.
    x11_ao_autenticar: Arc<AtomicBool>,
    /// O `exec` roda na hora, sem esperar o EOF, como num `sshd` de verdade (o `launch` não manda
    /// EOF); o comando que cita "x11" abre também um canal X11 para o cliente.
    exec_imediato: Arc<AtomicBool>,
    /// O servidor só oferece `none` na compressão (o padrão dele oferece também o `zlib`).
    sem_compressao: Arc<AtomicBool>,
    /// Quantas recusas de autenticação o servidor tolera antes de derrubar a conexão (o
    /// `MaxAuthTries` do OpenSSH); 0 deixa o padrão do `russh`.
    max_tentativas: Arc<AtomicUsize>,
    /// O shell despeja saída sem fim (o "despeja" digitado) até chegar um Ctrl+C.
    despejando: Arc<AtomicBool>,
}

impl Modo {
    /// Roda `comando` e devolve (status, stdout, stderr).
    fn rodar(&self, comando: &str, stdin: &[u8], log: &Log) -> (u32, Vec<u8>, Vec<u8>) {
        log.lock().unwrap().push(format!("exec {comando}"));
        if self.windows {
            self.rodar_windows(comando, stdin, log)
        } else {
            self.rodar_unix(comando, stdin)
        }
    }

    fn rodar_unix(&self, comando: &str, stdin: &[u8]) -> (u32, Vec<u8>, Vec<u8>) {
        let mut filho = std::process::Command::new("sh")
            .args(["-c", comando])
            .current_dir(&self.home)
            .env("HOME", &self.home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        // O `stdin` é pequeno: cabe no pipe, então escrever antes de ler não trava.
        let mut entrada = filho.stdin.take().unwrap();
        let _ = entrada.write_all(stdin);
        drop(entrada);
        let saida = filho.wait_with_output().unwrap();
        (
            saida.status.code().unwrap_or(255) as u32,
            saida.stdout,
            saida.stderr,
        )
    }

    fn rodar_windows(&self, comando: &str, stdin: &[u8], log: &Log) -> (u32, Vec<u8>, Vec<u8>) {
        if comando.starts_with("echo $0") {
            let erro = "'uname' não é reconhecido como um comando interno\r\n";
            return (1, b"$0\r\n".to_vec(), erro.as_bytes().to_vec());
        }
        if comando.contains("OSVersion.Platform") {
            return (0, b"Win32NT\r\n".to_vec(), Vec::new());
        }
        let Some(codificado) =
            comando.strip_prefix("powershell -NoProfile -NonInteractive -EncodedCommand ")
        else {
            return (1, Vec::new(), b"comando desconhecido".to_vec());
        };
        let script = decodificar_powershell(codificado);
        log.lock().unwrap().push(format!("script {script}"));
        if script.contains("Get-Content -Raw") {
            let caminho = if self.admin {
                "C:\\ProgramData\\ssh\\administrators_authorized_keys"
            } else {
                "C:\\Users\\ana\\.ssh\\authorized_keys"
            };
            let tipo = if self.admin { "admin" } else { "user" };
            let atual = self.arquivo.lock().unwrap().clone();
            let saida = format!("{tipo}\r\n{caminho}\r\n{atual}");
            return (0, saida.into_bytes(), Vec::new());
        }
        log.lock().unwrap().push(format!(
            "stdin {}",
            String::from_utf8_lossy(stdin).trim_end()
        ));
        let mut arquivo = self.arquivo.lock().unwrap();
        if !arquivo.is_empty() && !arquivo.ends_with('\n') {
            arquivo.push_str("\r\n");
        }
        arquivo.push_str(String::from_utf8_lossy(stdin).trim());
        arquivo.push_str("\r\n");
        (0, Vec::new(), Vec::new())
    }
}

/// O contrário do `-EncodedCommand`: base64 de UTF-16 little-endian.
fn decodificar_powershell(codificado: &str) -> String {
    let mut bits = 0u32;
    let mut quantos = 0;
    let mut bytes = Vec::new();
    for caractere in codificado.bytes().filter(|byte| *byte != b'=') {
        let valor = match caractere {
            b'A'..=b'Z' => caractere - b'A',
            b'a'..=b'z' => caractere - b'a' + 26,
            b'0'..=b'9' => caractere - b'0' + 52,
            b'+' => 62,
            _ => 63,
        };
        bits = (bits << 6) | u32::from(valor);
        quantos += 6;
        if quantos >= 8 {
            quantos -= 8;
            bytes.push((bits >> quantos) as u8);
            bits &= (1 << quantos) - 1;
        }
    }
    let unidades: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|par| u16::from_le_bytes([par[0], par[1]]))
        .collect();
    String::from_utf16_lossy(&unidades)
}

impl Servidor {
    fn registrar(&self, linha: String) {
        self.log.lock().unwrap().push(linha);
    }
}

impl server::Handler for Servidor {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "ana" && password == "senha" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(&mut self, user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        Ok(
            if user == "ana" && key.key_data() == self.client.key_data() {
                Auth::Accept
            } else {
                Auth::reject()
            },
        )
    }

    async fn channel_open_session(
        &mut self,
        _: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        _: ChannelId,
        term: &str,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.registrar(format!("pty {term} {cols}x{rows}"));
        self.pty = true;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.registrar("shell".into());
        self.shells.push(channel);
        session.data(channel, "bem-vinda, ana\r\n".as_bytes().to_vec())?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some((_, entrada)) = self.comandos.get_mut(&channel) {
            entrada.extend_from_slice(data);
            return Ok(());
        }
        if !self.shells.contains(&channel) {
            return Ok(());
        }
        if data == b"despeja\r" {
            self.modo.despejando.store(true, Ordering::SeqCst);
            let (despejando, handle) = (Arc::clone(&self.modo.despejando), session.handle());
            tokio::spawn(async move {
                let pedaco = b"linha de teste com bastante texto\r\n".repeat(1000);
                while despejando.load(Ordering::SeqCst) {
                    if handle.data(channel, pedaco.clone()).await.is_err() {
                        return;
                    }
                }
                let _ = handle.data(channel, b"fim-despejo".to_vec()).await;
            });
        } else if data == b"\x03" {
            self.registrar("ctrl-c".into());
            self.modo.despejando.store(false, Ordering::SeqCst);
        } else if data == b"exit 3\r" {
            session.exit_status_request(channel, 3)?;
            session.close(channel)?;
        } else if data == b"x11\r" || data == b"x11 errado\r" {
            let cookie = self.x11_cookie.clone().unwrap_or_default();
            let errado = data == b"x11 errado\r";
            tokio::spawn(abrir_x11(
                session.handle(),
                Arc::clone(&self.log),
                cookie,
                errado,
            ));
        } else {
            session.data(channel, data.to_vec())?;
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        if self.pty {
            self.registrar(format!("exec-pty {}", String::from_utf8_lossy(data)));
        }
        if self.modo.exec_imediato.load(Ordering::SeqCst) {
            let comando = String::from_utf8_lossy(data).into_owned();
            let handle = session.handle();
            if comando.contains("x11") {
                let cookie = self.x11_cookie.clone().unwrap_or_default();
                tokio::spawn(abrir_x11(
                    handle.clone(),
                    Arc::clone(&self.log),
                    cookie,
                    false,
                ));
            }
            let (modo, log) = (self.modo.clone(), Arc::clone(&self.log));
            tokio::spawn(async move {
                let (status, saida, erro) =
                    tokio::task::spawn_blocking(move || modo.rodar(&comando, b"", &log))
                        .await
                        .unwrap();
                if !saida.is_empty() {
                    let _ = handle.data(channel, saida).await;
                }
                if !erro.is_empty() {
                    let _ = handle.extended_data(channel, 1, erro).await;
                }
                let _ = handle.exit_status_request(channel, status).await;
                let _ = handle.eof(channel).await;
                let _ = handle.close(channel).await;
            });
            return Ok(());
        }
        self.comandos.insert(
            channel,
            (String::from_utf8_lossy(data).into_owned(), Vec::new()),
        );
        Ok(())
    }

    /// O cliente fechou a entrada: roda o comando e devolve a saída.
    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some((comando, entrada)) = self.comandos.remove(&channel) else {
            return Ok(());
        };
        let (status, saida, erro) = self.modo.rodar(&comando, &entrada, &self.log);
        if !saida.is_empty() {
            session.data(channel, saida)?;
        }
        if !erro.is_empty() {
            session.extended_data(channel, 1, erro)?;
        }
        session.exit_status_request(channel, status)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }

    /// O canal fechou: se ainda havia um comando esperando o EOF, o cliente desistiu no meio.
    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some((comando, entrada)) = self.comandos.remove(&channel) {
            self.registrar(format!(
                "fechou-sem-eof {comando} ({} bytes)",
                entrada.len()
            ));
        }
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        canal: Channel<Msg>,
        host: &str,
        porta: u32,
        _: &str,
        _: u32,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.registrar(format!("túnel {host}:{porta}"));
        if self.modo.bloqueia_tuneis.load(Ordering::SeqCst) {
            reply
                .reject(russh::ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        let Ok(porta) = u16::try_from(porta) else {
            return Ok(());
        };
        let Ok(mut destino) = tokio::net::TcpStream::connect((host, porta)).await else {
            reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            let mut canal = canal.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut canal, &mut destino).await;
        });
        Ok(())
    }

    #[cfg(unix)]
    async fn channel_open_direct_streamlocal(
        &mut self,
        canal: Channel<Msg>,
        caminho: &str,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.registrar(format!("socket {caminho}"));
        let Ok(mut destino) = tokio::net::UnixStream::connect(caminho).await else {
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            let mut canal = canal.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut canal, &mut destino).await;
        });
        Ok(())
    }

    async fn x11_request(
        &mut self,
        canal: ChannelId,
        unica: bool,
        protocolo: &str,
        cookie: &str,
        tela: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.registrar(format!("x11 {protocolo} {cookie} {tela} {unica}"));
        if self.modo.recusa_x11.load(Ordering::SeqCst) {
            session.channel_failure(canal)?;
        } else {
            self.x11_cookie = Some(
                (0..cookie.len() / 2)
                    .filter_map(|i| u8::from_str_radix(&cookie[i * 2..i * 2 + 2], 16).ok())
                    .collect(),
            );
            session.channel_success(canal)?;
        }
        Ok(())
    }

    async fn auth_succeeded(&mut self, session: &mut Session) -> Result<(), Self::Error> {
        if self.modo.x11_ao_autenticar.load(Ordering::SeqCst) {
            tokio::spawn(abrir_x11(
                session.handle(),
                Arc::clone(&self.log),
                vec![0; 16],
                false,
            ));
        }
        Ok(())
    }

    async fn tcpip_forward(
        &mut self,
        endereco: &str,
        porta: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.registrar(format!("tcpip-forward {endereco}:{porta}"));
        if self.modo.bloqueia_tuneis.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let Ok(porta_pedida) = u16::try_from(*porta) else {
            return Ok(false);
        };
        let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", porta_pedida)).await else {
            return Ok(false);
        };
        let real = u32::from(listener.local_addr().unwrap().port());
        *porta = real;
        let (handle, endereco) = (session.handle(), endereco.to_owned());
        let tarefa = tokio::spawn(async move {
            while let Ok((mut entrada, origem)) = listener.accept().await {
                let (handle, endereco) = (handle.clone(), endereco.clone());
                tokio::spawn(async move {
                    let Ok(canal) = handle
                        .channel_open_forwarded_tcpip(
                            endereco,
                            real,
                            origem.ip().to_string(),
                            u32::from(origem.port()),
                        )
                        .await
                    else {
                        return;
                    };
                    let mut canal = canal.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut entrada, &mut canal).await;
                });
            }
        });
        self.remotas.insert(real, tarefa);
        Ok(true)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        endereco: &str,
        porta: u32,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.registrar(format!("cancel-tcpip-forward {endereco}:{porta}"));
        match self.remotas.remove(&porta) {
            Some(tarefa) => {
                tarefa.abort();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn agent_request(
        &mut self,
        _: ChannelId,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        // Abrir o canal espera a resposta do cliente, que só chega com o laço da sessão livre.
        let (handle, log) = (session.handle(), Arc::clone(&self.log));
        tokio::spawn(async move {
            let linha = match handle.channel_open_agent().await {
                Ok(canal) => {
                    let mut agente = AgentClient::connect(canal.into_stream());
                    match agente.request_identities().await {
                        Ok(chaves) => format!("agente com {} chave(s)", chaves.len()),
                        Err(erro) => format!("agente com erro: {erro}"),
                    }
                }
                Err(erro) => format!("agente recusado: {erro}"),
            };
            log.lock().unwrap().push(linha);
        });
        Ok(true)
    }

    async fn window_change_request(
        &mut self,
        _: ChannelId,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.registrar(format!("janela {cols}x{rows}"));
        Ok(())
    }
}

/// O lado do servidor de um programa gráfico: abre um canal X11 para o cliente, manda o setup
/// com o `cookie`, espera o "ok" do servidor X (de mentira) e confere o eco depois dele.
async fn abrir_x11(handle: server::Handle, log: Log, cookie: Vec<u8>, errado: bool) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let canal = match handle.channel_open_x11("127.0.0.1", 4000).await {
        Ok(canal) => canal,
        Err(_) => {
            log.lock().unwrap().push("x11 recusado".into());
            return;
        }
    };
    let mut canal = canal.into_stream();
    let nome = b"MIT-MAGIC-COOKIE-1";
    let dado = if errado { vec![0xEE; 16] } else { cookie };
    let mut pacote = vec![
        b'l',
        0,
        11,
        0,
        0,
        0,
        nome.len() as u8,
        0,
        dado.len() as u8,
        0,
        0,
        0,
    ];
    pacote.extend_from_slice(nome);
    pacote.resize(12 + nome.len().next_multiple_of(4), 0);
    pacote.extend_from_slice(&dado);
    pacote.resize(
        12 + nome.len().next_multiple_of(4) + dado.len().next_multiple_of(4),
        0,
    );
    let _ = canal.write_all(&pacote).await;
    let mut resposta = [0u8; 2];
    if canal.read_exact(&mut resposta).await.is_err() {
        log.lock().unwrap().push("x11 fechado".into());
        return;
    }
    let _ = canal.write_all(b"ping").await;
    let mut eco = [0u8; 4];
    let linha = match canal.read_exact(&mut eco).await {
        Ok(_) => format!("x11 ok {}", String::from_utf8_lossy(&resposta)),
        Err(_) => "x11 sem eco".to_owned(),
    };
    log.lock().unwrap().push(linha);
}

struct Teste {
    porta: u16,
    servidor: PublicKey,
    cliente: PrivateKey,
    log: Log,
    pasta: PathBuf,
    /// Quantos clientes o servidor já aceitou.
    conexoes: Arc<AtomicUsize>,
    modo: Modo,
}

impl Teste {
    /// Sobe o servidor numa thread e grava a chave dele num `known_hosts` próprio.
    fn novo(nome: &str) -> Self {
        let cliente = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        Self::com_cliente(nome, cliente)
    }

    /// Como `novo`, com a chave que o servidor aceita do cliente.
    fn com_cliente(nome: &str, cliente: PrivateKey) -> Self {
        Self::com_modo(nome, cliente, false, false)
    }

    /// Como `novo`, com um servidor Windows simulado (`admin`: o usuário é administrador).
    fn windows(nome: &str, admin: bool) -> Self {
        let cliente = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        Self::com_modo(nome, cliente, true, admin)
    }

    fn com_modo(nome: &str, cliente: PrivateKey, windows: bool, admin: bool) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("tomni-ui-ssh-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&pasta);
        let home = pasta.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let modo = Modo {
            windows,
            admin,
            home,
            arquivo: Arc::default(),
            bloqueia_tuneis: Arc::default(),
            recusa_x11: Arc::default(),
            x11_ao_autenticar: Arc::default(),
            exec_imediato: Arc::default(),
            sem_compressao: Arc::default(),
            max_tentativas: Arc::default(),
            despejando: Arc::default(),
        };
        let chave = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let servidor = chave.public_key().clone();
        let log = Log::default();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let porta = listener.local_addr().unwrap().port();
        let conexoes = Arc::new(AtomicUsize::new(0));
        let (aceitas, registro, publica, modo_servidor) = (
            Arc::clone(&conexoes),
            Arc::clone(&log),
            cliente.public_key().clone(),
            modo.clone(),
        );
        thread::spawn(move || {
            while let Ok((stream, _)) = listener.accept() {
                aceitas.fetch_add(1, Ordering::SeqCst);
                let sem_compressao = Arc::clone(&modo_servidor.sem_compressao);
                let max_tentativas = modo_servidor.max_tentativas.load(Ordering::SeqCst);
                let handler = Servidor {
                    client: publica.clone(),
                    log: Arc::clone(&registro),
                    shells: Vec::new(),
                    modo: modo_servidor.clone(),
                    comandos: Default::default(),
                    pty: false,
                    x11_cookie: None,
                    remotas: Default::default(),
                };
                let chave = chave.clone();
                // Uma thread por cliente: cada uma com o seu runtime, como o `Ssh` faz.
                thread::spawn(move || {
                    stream.set_nonblocking(true).unwrap();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    runtime.block_on(async move {
                        let mut preferred = russh::Preferred::default();
                        if sem_compressao.load(Ordering::SeqCst) {
                            preferred.compression =
                                std::borrow::Cow::Borrowed(&[russh::compression::NONE]);
                        }
                        let padrao = server::Config::default().max_auth_attempts;
                        let config = Arc::new(server::Config {
                            preferred,
                            max_auth_attempts: if max_tentativas == 0 {
                                padrao
                            } else {
                                max_tentativas
                            },
                            keys: vec![chave],
                            auth_rejection_time: Duration::ZERO,
                            auth_rejection_time_initial: Some(Duration::ZERO),
                            ..Default::default()
                        });
                        let stream = tokio::net::TcpStream::from_std(stream).unwrap();
                        if let Ok(sessao) = server::run_stream(config, stream, handler).await {
                            let _ = sessao.await;
                        }
                    });
                });
            }
        });
        Self {
            porta,
            servidor,
            cliente,
            log,
            pasta,
            conexoes,
            modo,
        }
    }

    /// A pasta pessoal do servidor Unix de teste.
    #[cfg_attr(not(unix), allow(dead_code))]
    fn home(&self) -> &Path {
        &self.modo.home
    }

    fn known_hosts(&self) -> PathBuf {
        self.pasta.join("known_hosts")
    }

    fn conhecer_servidor(&self) {
        learn_known_hosts_path("127.0.0.1", self.porta, &self.servidor, self.known_hosts())
            .unwrap();
    }

    fn conexoes(&self) -> usize {
        self.conexoes.load(Ordering::SeqCst)
    }

    fn builder(&self) -> SshBuilder {
        Ssh::builder("127.0.0.1", self.porta, "ana").known_hosts(self.known_hosts())
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

/// Lê até `fim` aparecer ou a sessão fechar, com um limite de 10 s.
fn ler_ate(ssh: &mut Ssh, fim: &str) -> (String, Status) {
    let limite = Instant::now() + Duration::from_secs(10);
    let mut lido = Vec::new();
    loop {
        let status = ssh.read(&mut lido, usize::MAX);
        let texto = String::from_utf8_lossy(&lido).into_owned();
        if texto.contains(fim) || status != Status::Open || Instant::now() > limite {
            return (texto, status);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn esperar(condicao: impl Fn() -> bool) {
    let limite = Instant::now() + Duration::from_secs(10);
    while !condicao() && Instant::now() < limite {
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn senha_shell_eco_e_tamanho() {
    let teste = Teste::novo("senha");
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("senha").connect();
    ssh.resize(100, 30);
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda, ana\r\n");
    assert_eq!(status, Status::Open, "{texto}");
    ssh.write("olá".as_bytes()).unwrap();
    let (texto, _) = ler_ate(&mut ssh, "olá");
    assert!(texto.ends_with("olá"), "{texto:?}");
    ssh.resize(120, 40);
    esperar(|| teste.log().contains(&"janela 120x40".to_owned()));
    // A conexão roda numa thread só, a que o app mede.
    let threads = ssh.threads();
    assert_eq!(
        threads.len(),
        usize::from(cfg!(target_os = "linux")),
        "{threads:?}"
    );
    let log = teste.log();
    assert_eq!(log[0], "pty xterm-256color 100x30", "{log:?}");
    assert_eq!(log[1], "shell");
    assert!(log.contains(&"janela 120x40".to_owned()), "{log:?}");
}

#[cfg(unix)]
#[test]
fn saida_sem_fim_nao_passa_do_limite_da_fila_e_o_ctrl_c_chega() {
    let teste = Teste::novo("despejo");
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("senha").connect();
    ler_ate(&mut ssh, "bem-vinda");
    ssh.write(b"despeja\r").unwrap();
    esperar(|| ssh.inbox.queued() >= crate::INBOX_LIMIT);
    thread::sleep(Duration::from_millis(300));
    let fila = ssh.inbox.queued();
    assert!(
        fila >= crate::INBOX_LIMIT,
        "o servidor enche a fila: {fila}"
    );
    // A folga depois da tecla (`KEY_SLACK`) passa do limite em 1 MiB, no máximo.
    assert!(
        fila < 2 * crate::INBOX_LIMIT + 64 * 1024,
        "e para no limite: {fila}"
    );
    // Um canal novo na mesma conexão (o SFTP, um comando) abre com a fila cheia.
    let canais = ssh.ssh_channels().unwrap();
    let comando = thread::spawn(move || {
        canais.exec(
            "echo canal-extra",
            &mut std::io::empty(),
            &mut |_| {},
            Duration::from_secs(20),
        )
    });
    thread::sleep(Duration::from_millis(300));
    // Sem ninguém ler, as teclas ainda vão.
    ssh.write(b"\x03").unwrap();
    esperar(|| teste.log().contains(&"ctrl-c".to_owned()));
    assert!(
        teste.log().contains(&"ctrl-c".to_owned()),
        "{:?}",
        teste.log()
    );
    let limite = Instant::now() + Duration::from_secs(20);
    let mut lido = Vec::new();
    loop {
        lido.drain(..lido.len().saturating_sub(16));
        assert_eq!(ssh.read(&mut lido, usize::MAX), Status::Open);
        if lido.windows(11).any(|w| w == b"fim-despejo") {
            break;
        }
        assert!(Instant::now() < limite, "o despejo não terminou");
        thread::sleep(Duration::from_millis(1));
    }
    let saida = comando.join().unwrap().unwrap();
    assert_eq!(saida.stdout_text(), "canal-extra\n");
}

#[test]
fn chave_privada_num_arquivo() {
    let teste = Teste::novo("chave");
    teste.conhecer_servidor();
    let arquivo = teste.pasta.join("id_ed25519");
    let pem = teste.cliente.to_openssh(LineEnding::LF).unwrap();
    std::fs::write(&arquivo, pem.as_bytes()).unwrap();
    let mut ssh = teste.builder().key_file(&arquivo, None).connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
    assert!(texto.contains("bem-vinda, ana"), "{texto:?}");
}

#[test]
fn chave_privada_na_memoria_sem_tocar_o_disco() {
    let teste = Teste::novo("chave-memoria");
    teste.conhecer_servidor();
    let mut ssh = teste.builder().key(teste.cliente.clone()).connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
    assert!(texto.contains("bem-vinda, ana"), "{texto:?}");
    // O `Debug` do builder não mostra a chave.
    let builder = format!("{:?}", teste.builder().key(teste.cliente.clone()));
    assert!(builder.contains("na memória"), "{builder}");
    assert!(!builder.contains("PRIVATE"), "{builder}");
}

#[test]
fn senha_errada() {
    let teste = Teste::novo("senha-errada");
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("outra").connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert_eq!(
        status,
        Status::Closed(Some("autenticação recusada para ana".into()))
    );
}

#[test]
fn servidor_desconhecido_e_recusado_por_padrao() {
    let teste = Teste::novo("desconhecido");
    let mut ssh = teste.builder().password("senha").connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    let Status::Closed(Some(mensagem)) = status else {
        panic!("devia recusar: {status:?}");
    };
    let impressao = teste.servidor.fingerprint(HashAlg::Sha256).to_string();
    assert_eq!(
        mensagem,
        format!("a chave do servidor não é conhecida: ssh-ed25519 {impressao}")
    );
}

#[test]
fn o_app_aceita_e_grava_a_chave() {
    let teste = Teste::novo("aceita");
    let vistas = Arc::new(Mutex::new(Vec::new()));
    let registro = Arc::clone(&vistas);
    let mut ssh = teste
        .builder()
        .password("senha")
        .check_host_key(move |chave, conhecida| {
            registro
                .lock()
                .unwrap()
                .push((chave.fingerprint(), conhecida));
            chave.remember().is_ok()
        })
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
    let impressao = teste.servidor.fingerprint(HashAlg::Sha256).to_string();
    assert_eq!(*vistas.lock().unwrap(), [(impressao, KnownHost::Unknown)]);
    assert!(
        check_known_hosts_path(
            "127.0.0.1",
            teste.porta,
            &teste.servidor,
            teste.known_hosts()
        )
        .unwrap()
    );
}

#[test]
fn chave_que_mudou() {
    let teste = Teste::novo("mudou");
    // Num arquivo vazio, o `russh` começa com uma linha em branco: a chave fica na linha 2.
    let outra = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    learn_known_hosts_path(
        "127.0.0.1",
        teste.porta,
        outra.public_key(),
        teste.known_hosts(),
    )
    .unwrap();
    let mut ssh = teste.builder().password("senha").connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    let Status::Closed(Some(mensagem)) = status else {
        panic!("devia recusar: {status:?}");
    };
    assert!(
        mensagem
            .starts_with("a chave do servidor mudou (linha 2 do known_hosts): ssh-ed25519 SHA256:"),
        "{mensagem}"
    );
}

#[test]
fn saida_com_codigo() {
    let teste = Teste::novo("saida");
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("senha").connect();
    ler_ate(&mut ssh, "bem-vinda");
    ssh.write(b"exit 3\r").unwrap();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert_eq!(
        status,
        Status::Closed(Some("o programa remoto saiu com o código 3".into()))
    );
}

#[test]
fn conexao_recusada() {
    let porta = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut ssh = Ssh::builder("127.0.0.1", porta, "ana").connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert_eq!(status, Status::Closed(Some("conexão recusada".into())));
}

/// Liga dois sockets até um deles fechar.
fn ponte(a: TcpStream, b: TcpStream) {
    for (mut origem, mut destino) in [(a.try_clone().unwrap(), b.try_clone().unwrap()), (b, a)] {
        thread::spawn(move || {
            let _ = std::io::copy(&mut origem, &mut destino);
            let _ = destino.shutdown(std::net::Shutdown::Both);
        });
    }
}

/// Um relé TCP até `destino`; depois de `parar`, ele lê e joga fora, sem fechar nada: a rede
/// que "cai" sem avisar.
fn rele(destino: u16, parar: Arc<AtomicBool>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (cliente, _) = listener.accept().unwrap();
        let servidor = TcpStream::connect(("127.0.0.1", destino)).unwrap();
        for (mut origem, mut destino) in [
            (cliente.try_clone().unwrap(), servidor.try_clone().unwrap()),
            (servidor, cliente),
        ] {
            let parar = Arc::clone(&parar);
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(lidos) = origem.read(&mut buf) {
                    if lidos == 0 {
                        break;
                    }
                    if !parar.load(Ordering::SeqCst) {
                        let _ = destino.write_all(&buf[..lidos]);
                    }
                }
            });
        }
    });
    porta
}

#[test]
fn restart_reabre_e_duplicate_abre_outra() {
    let teste = Teste::novo("restart");
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("senha").connect();
    ler_ate(&mut ssh, "bem-vinda");
    ssh.write(b"exit 3\r").unwrap();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert!(matches!(status, Status::Closed(Some(_))), "{status:?}");

    assert!(ssh.reopen());
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda, ana\r\n");
    assert_eq!(status, Status::Open, "{texto}");
    ssh.write(b"eco").unwrap();
    let (texto, _) = ler_ate(&mut ssh, "eco");
    assert!(texto.ends_with("eco"), "{texto:?}");
    assert_eq!(teste.conexoes(), 2);

    let mut outra = ssh.duplicate().expect("o Ssh duplica");
    let (texto, status) = ler_ate_transport(outra.as_mut(), "bem-vinda, ana\r\n");
    assert_eq!(status, Status::Open, "{texto}");
    assert_eq!(teste.conexoes(), 3);
}

#[test]
fn duplicate_running_troca_o_comando_da_copia() {
    let teste = Teste::novo("duplicar-comando");
    teste.conhecer_servidor();
    let ssh = teste
        .builder()
        .password("senha")
        .command("tmux new-session -A -s a")
        .connect();
    esperar(|| {
        teste
            .log()
            .iter()
            .any(|l| l == "exec-pty tmux new-session -A -s a")
    });
    let _outra = ssh
        .duplicate_running(Some("tmux new-session -A -s b"))
        .expect("o Ssh duplica");
    esperar(|| {
        teste
            .log()
            .iter()
            .any(|l| l == "exec-pty tmux new-session -A -s b")
    });
    let mut shell = ssh.duplicate_running(None).expect("o Ssh duplica");
    let (texto, status) = ler_ate_transport(shell.as_mut(), "bem-vinda, ana\r\n");
    assert_eq!(status, Status::Open, "{texto}");
    assert_eq!(teste.conexoes(), 3);
}

fn ler_ate_transport(transport: &mut dyn Transport, fim: &str) -> (String, Status) {
    let limite = Instant::now() + Duration::from_secs(10);
    let mut lido = Vec::new();
    loop {
        let status = transport.read(&mut lido, usize::MAX);
        let texto = String::from_utf8_lossy(&lido).into_owned();
        if texto.contains(fim) || status != Status::Open || Instant::now() > limite {
            return (texto, status);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn term_e_pedido_no_pseudoterminal() {
    let teste = Teste::novo("term");
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("senha").term("vt100").connect();
    ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(teste.log()[0], "pty vt100 80x24");
}

#[test]
fn jump_com_tres_saltos() {
    let (j1, j2, destino) = (
        Teste::novo("salto-1"),
        Teste::novo("salto-2"),
        Teste::novo("destino"),
    );
    for t in [&j1, &j2, &destino] {
        t.conhecer_servidor();
    }
    let mut ssh = destino
        .builder()
        .password("senha")
        .jump(
            j2.builder()
                .password("senha")
                .jump(j1.builder().password("senha")),
        )
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda, ana\r\n");
    assert_eq!(status, Status::Open, "{texto}");
    ssh.write(b"oi").unwrap();
    let (texto, _) = ler_ate(&mut ssh, "oi");
    assert!(texto.ends_with("oi"), "{texto:?}");

    // Cada salto abriu o túnel até o seguinte, e só o destino abriu o shell.
    assert_eq!(j1.log(), [format!("túnel 127.0.0.1:{}", j2.porta)]);
    assert_eq!(j2.log(), [format!("túnel 127.0.0.1:{}", destino.porta)]);
    assert_eq!(destino.log()[0], "pty xterm-256color 80x24");
}

#[test]
fn jump_com_chave_trocada_cita_o_salto() {
    let (j1, j2, destino) = (
        Teste::novo("troca-1"),
        Teste::novo("troca-2"),
        Teste::novo("troca-destino"),
    );
    j1.conhecer_servidor();
    destino.conhecer_servidor();
    // O j2 está no `known_hosts` com outra chave.
    let outra = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    learn_known_hosts_path("127.0.0.1", j2.porta, outra.public_key(), j2.known_hosts()).unwrap();
    let mut ssh = destino
        .builder()
        .password("senha")
        .jump(
            j2.builder()
                .password("senha")
                .jump(j1.builder().password("senha")),
        )
        .connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    let Status::Closed(Some(mensagem)) = status else {
        panic!("devia recusar: {status:?}");
    };
    let esperado = format!("127.0.0.1:{}: a chave do servidor mudou", j2.porta);
    assert!(mensagem.starts_with(&esperado), "{mensagem}");
    assert!(destino.log().is_empty());
}

#[test]
fn jump_para_um_destino_que_o_salto_nao_alcanca() {
    let salto = Teste::novo("sem-rota-salto");
    salto.conhecer_servidor();
    let porta = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut ssh = Ssh::builder("127.0.0.1", porta, "ana")
        .jump(salto.builder().password("senha"))
        .connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    let Status::Closed(Some(mensagem)) = status else {
        panic!("devia falhar: {status:?}");
    };
    assert!(
        mensagem.starts_with(&format!("127.0.0.1:{}: ", salto.porta)),
        "{mensagem}"
    );
    assert!(
        mensagem.contains(&format!("túnel até 127.0.0.1:{porta}")),
        "{mensagem}"
    );
}

#[test]
fn keepalive_detecta_a_queda() {
    let teste = Teste::novo("keepalive");
    let parar = Arc::new(AtomicBool::new(false));
    let porta = rele(teste.porta, Arc::clone(&parar));
    learn_known_hosts_path("127.0.0.1", porta, &teste.servidor, teste.known_hosts()).unwrap();
    let mut ssh = Ssh::builder("127.0.0.1", porta, "ana")
        .known_hosts(teste.known_hosts())
        .password("senha")
        .keepalive(Duration::from_millis(200))
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");

    parar.store(true, Ordering::SeqCst);
    let inicio = Instant::now();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert_eq!(
        status,
        Status::Closed(Some("tempo de conexão esgotado".into()))
    );
    assert!(
        inicio.elapsed() < Duration::from_secs(5),
        "{:?}",
        inicio.elapsed()
    );
}

/// Um agente de chaves num socket Unix, com a chave `chave` dentro.
#[cfg(unix)]
fn servir_agente(socket: &Path, chave: &PrivateKey) {
    let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let (socket, chave) = (socket.to_owned(), chave.clone());
    let (pronto, espera) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::UnixListener::from_std(listener).unwrap();
            let conexoes = futures::stream::poll_fn(move |cx| {
                listener
                    .poll_accept(cx)
                    .map(|aceita| Some(aceita.map(|(stream, _)| stream)))
            });
            tokio::spawn(russh::keys::agent::server::serve(Box::pin(conexoes), ()));
            let mut cliente = AgentClient::connect_uds(&socket).await.unwrap();
            cliente.add_identity(&chave, &[]).await.unwrap();
            pronto.send(()).unwrap();
            std::future::pending::<()>().await;
        });
    });
    espera.recv_timeout(Duration::from_secs(10)).unwrap();
}

#[cfg(unix)]
#[test]
fn agente_local_autentica_sem_senha() {
    let teste = Teste::novo("agente");
    teste.conhecer_servidor();
    let socket = teste.pasta.join("agente.sock");
    servir_agente(&socket, &teste.cliente);
    // Sem senha e sem arquivo de chave: só o agente.
    let mut ssh = teste.builder().agent_auth().agent_path(&socket).connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
}

#[test]
fn agente_que_nao_existe_nao_e_erro() {
    let teste = Teste::novo("sem-agente");
    teste.conhecer_servidor();
    let socket = teste.pasta.join("nao-existe.sock");
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(&socket)
        .password("senha")
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
}

#[cfg(unix)]
#[test]
fn encaminha_o_agente() {
    let teste = Teste::novo("encaminha");
    teste.conhecer_servidor();
    let socket = teste.pasta.join("agente.sock");
    servir_agente(&socket, &teste.cliente);
    let mut ssh = teste
        .builder()
        .password("senha")
        .agent_path(&socket)
        .agent_forward(true)
        .connect();
    ler_ate(&mut ssh, "bem-vinda");
    esperar(|| teste.log().iter().any(|linha| linha.starts_with("agente")));
    assert!(
        teste.log().contains(&"agente com 1 chave(s)".to_owned()),
        "{:?}",
        teste.log()
    );
}

#[cfg(unix)]
#[test]
fn sem_agent_forward_o_servidor_nao_alcanca_o_agente() {
    let teste = Teste::novo("sem-encaminhar");
    teste.conhecer_servidor();
    let socket = teste.pasta.join("agente.sock");
    servir_agente(&socket, &teste.cliente);
    let mut ssh = teste
        .builder()
        .password("senha")
        .agent_path(&socket)
        .connect();
    ler_ate(&mut ssh, "bem-vinda");
    thread::sleep(Duration::from_millis(300));
    assert!(
        !teste.log().iter().any(|linha| linha.starts_with("agente")),
        "{:?}",
        teste.log()
    );
}

/// O agente do Tomni UI (o `ssh::agent`) com a chave `chave`, num socket da pasta do teste.
#[cfg(unix)]
fn nosso_agente(teste: &Teste, chave: &PrivateKey) -> agent::Agent {
    let agente = agent::Agent::start(Some(teste.pasta.join("nosso.sock"))).unwrap();
    agente
        .add(chave.clone(), "teste", agent::Constraints::default())
        .expect("guardar");
    agente
}

#[cfg(unix)]
#[test]
fn o_ssh_autentica_pelo_agente_do_tomni_ui_com_ed25519() {
    let teste = Teste::novo("nosso-agente-ed25519");
    teste.conhecer_servidor();
    let agente = nosso_agente(&teste, &teste.cliente);
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(agente.path())
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
}

#[cfg(unix)]
#[test]
fn o_ssh_autentica_pelo_agente_do_tomni_ui_com_rsa_sha2() {
    let rsa = keys::KeyGen::new(keys::KeyType::Rsa(keys::RsaBits::B3072))
        .generate()
        .unwrap();
    let teste = Teste::com_cliente("nosso-agente-rsa", rsa.private_key().clone());
    teste.conhecer_servidor();
    let agente = nosso_agente(&teste, &teste.cliente);
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(agente.path())
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
}

#[cfg(unix)]
#[test]
fn agente_travado_ou_sem_a_chave_nao_autentica() {
    let teste = Teste::novo("nosso-agente-travado");
    teste.conhecer_servidor();
    let agente = nosso_agente(&teste, &teste.cliente);
    assert!(agente.lock("segredo"));
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(agente.path())
        .connect();
    let (_, status) = ler_ate(&mut ssh, "bem-vinda");
    assert!(matches!(status, Status::Closed(_)), "{status:?}");

    // Destravado, a chave vale de novo.
    assert!(agente.unlock("segredo"));
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(agente.path())
        .connect();
    assert_eq!(ler_ate(&mut ssh, "bem-vinda").1, Status::Open);
}

/// O agente do Tomni UI com 8 chaves que o servidor não aceita e, por último, a certa.
#[cfg(unix)]
fn agente_cheio(teste: &Teste) -> agent::Agent {
    let agente = agent::Agent::start(Some(teste.pasta.join("cheio.sock"))).unwrap();
    for n in 0..8 {
        let outra = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        agente
            .add(outra, &format!("outra-{n}"), agent::Constraints::default())
            .expect("guardar");
    }
    agente
        .add(
            teste.cliente.clone(),
            "certa",
            agent::Constraints::default(),
        )
        .expect("guardar");
    agente
}

#[cfg(unix)]
#[test]
fn com_muitas_chaves_no_agente_a_da_conexao_vai_primeiro() {
    let teste = Teste::novo("agente-cheio");
    teste.modo.max_tentativas.store(6, Ordering::SeqCst);
    teste.conhecer_servidor();
    let agente = agente_cheio(&teste);

    // Sem saber qual é a chave, as 8 erradas esgotam o limite do servidor antes da certa, e a
    // mensagem diz isso (não que o agente recusou).
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(agente.path())
        .connect();
    let (_, status) = ler_ate(&mut ssh, "bem-vinda");
    let Status::Closed(Some(erro)) = status else {
        panic!("{status:?}");
    };
    assert!(erro.contains("MaxAuthTries"), "{erro}");
    assert!(!erro.contains("agente recusou"), "{erro}");

    // Com a chave ligada à conexão (a do cofre), a cópia dela no agente vai primeiro.
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(agente.path())
        .key(teste.cliente.clone())
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");

    // Pelo arquivo cifrado, sem a senha: a pública vai às claras, e o agente assina.
    let arquivo = teste.pasta.join("id_cifrada");
    let cifrada = teste
        .cliente
        .encrypt(&mut rand::rng(), "segredo")
        .unwrap()
        .to_openssh(LineEnding::LF)
        .unwrap();
    std::fs::write(&arquivo, cifrada.as_bytes()).unwrap();
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(agente.path())
        .key_file(&arquivo, None)
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
}

#[cfg(unix)]
#[test]
fn um_arquivo_de_chave_que_falta_nao_impede_o_agente() {
    let teste = Teste::novo("arquivo-que-falta");
    teste.conhecer_servidor();
    let agente = nosso_agente(&teste, &teste.cliente);
    // O caminho de outro computador (veio de uma importação) não existe aqui.
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(agente.path())
        .key_file(teste.pasta.join("nao-existe"), None)
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");

    // Sem o agente, o erro é o do arquivo.
    let mut ssh = teste
        .builder()
        .key_file(teste.pasta.join("nao-existe"), None)
        .connect();
    let (_, status) = ler_ate(&mut ssh, "bem-vinda");
    let Status::Closed(Some(erro)) = status else {
        panic!("{status:?}");
    };
    assert!(erro.contains("nao-existe"), "{erro}");
}

#[cfg(unix)]
#[test]
fn a_chave_com_confirmacao_so_autentica_se_for_permitida() {
    let teste = Teste::novo("nosso-agente-confirma");
    teste.conhecer_servidor();
    let agente = agent::Agent::start(Some(teste.pasta.join("nosso.sock"))).unwrap();
    agente
        .add(
            teste.cliente.clone(),
            "pede",
            agent::Constraints {
                lifetime: None,
                confirm: true,
            },
        )
        .expect("guardar");
    // Sem quem responda, o uso é negado.
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(agente.path())
        .connect();
    assert!(matches!(
        ler_ate(&mut ssh, "bem-vinda").1,
        Status::Closed(_)
    ));

    agente.on_confirm(|_| true);
    let mut ssh = teste
        .builder()
        .agent_auth()
        .agent_path(agente.path())
        .connect();
    assert_eq!(ler_ate(&mut ssh, "bem-vinda").1, Status::Open);
}

/// O que o proxy de teste viu: o destino pedido.
type Pedidos = Arc<Mutex<Vec<String>>>;

/// Um proxy SOCKS5 de teste; com `credenciais`, exige usuário e senha.
fn proxy_socks5(credenciais: Option<(&'static str, &'static str)>) -> (u16, Pedidos) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    let pedidos = Pedidos::default();
    let registro = Arc::clone(&pedidos);
    thread::spawn(move || {
        let (mut cliente, _) = listener.accept().unwrap();
        let mut cabecalho = [0u8; 2];
        cliente.read_exact(&mut cabecalho).unwrap();
        let mut metodos = vec![0u8; usize::from(cabecalho[1])];
        cliente.read_exact(&mut metodos).unwrap();
        if let Some((usuario, senha)) = credenciais {
            assert!(metodos.contains(&2), "{metodos:?}");
            cliente.write_all(&[5, 2]).unwrap();
            let mut versao_e_tamanho = [0u8; 2];
            cliente.read_exact(&mut versao_e_tamanho).unwrap();
            let mut lido_usuario = vec![0u8; usize::from(versao_e_tamanho[1])];
            cliente.read_exact(&mut lido_usuario).unwrap();
            let mut tamanho = [0u8; 1];
            cliente.read_exact(&mut tamanho).unwrap();
            let mut lida_senha = vec![0u8; usize::from(tamanho[0])];
            cliente.read_exact(&mut lida_senha).unwrap();
            if lido_usuario != usuario.as_bytes() || lida_senha != senha.as_bytes() {
                cliente.write_all(&[1, 1]).unwrap();
                return;
            }
            cliente.write_all(&[1, 0]).unwrap();
        } else {
            cliente.write_all(&[5, 0]).unwrap();
        }
        let mut pedido = [0u8; 5];
        cliente.read_exact(&mut pedido).unwrap();
        assert_eq!(&pedido[..4], [5, 1, 0, 3]);
        let mut host = vec![0u8; usize::from(pedido[4])];
        cliente.read_exact(&mut host).unwrap();
        let mut porta = [0u8; 2];
        cliente.read_exact(&mut porta).unwrap();
        let host = String::from_utf8(host).unwrap();
        let porta = u16::from_be_bytes(porta);
        registro.lock().unwrap().push(format!("{host}:{porta}"));
        let destino = TcpStream::connect((host.as_str(), porta)).unwrap();
        cliente.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        ponte(cliente, destino);
    });
    (porta, pedidos)
}

/// Um proxy HTTP CONNECT de teste; com `credenciais`, exige o `Proxy-Authorization: Basic`.
fn proxy_http(credenciais: Option<(&'static str, &'static str)>) -> (u16, Pedidos) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    let pedidos = Pedidos::default();
    let registro = Arc::clone(&pedidos);
    thread::spawn(move || {
        let (mut cliente, _) = listener.accept().unwrap();
        let mut cabecalho = Vec::new();
        let mut byte = [0u8; 1];
        while !cabecalho.ends_with(b"\r\n\r\n") {
            cliente.read_exact(&mut byte).unwrap();
            cabecalho.push(byte[0]);
        }
        let texto = String::from_utf8(cabecalho).unwrap();
        let alvo = texto.split_whitespace().nth(1).unwrap().to_owned();
        registro.lock().unwrap().push(alvo.clone());
        if let Some((usuario, senha)) = credenciais {
            let esperado = format!(
                "Proxy-Authorization: Basic {}",
                tomni_ui_net::base64(format!("{usuario}:{senha}").as_bytes())
            );
            if !texto.lines().any(|linha| linha == esperado) {
                cliente
                    .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                    .unwrap();
                return;
            }
        }
        let destino = TcpStream::connect(alvo.as_str()).unwrap();
        cliente
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .unwrap();
        ponte(cliente, destino);
    });
    (porta, pedidos)
}

fn proxy_socks5_para(porta: u16, usuario: Option<&str>, senha: Option<&str>) -> Proxy {
    Proxy::Socks5 {
        host: "127.0.0.1".into(),
        port: porta,
        user: usuario.map(str::to_owned),
        password: senha.map(str::to_owned),
    }
}

fn proxy_http_para(porta: u16, usuario: Option<&str>, senha: Option<&str>) -> Proxy {
    Proxy::Http {
        host: "127.0.0.1".into(),
        port: porta,
        user: usuario.map(str::to_owned),
        password: senha.map(str::to_owned),
    }
}

/// Um proxy SOCKS5 de teste que **ignora o destino pedido** e liga sempre a `destino`: uma
/// rede de cliente onde o `10.0.0.5` é outro computador. Registra o que foi pedido.
fn proxy_socks5_fixo(destino: u16) -> (u16, Pedidos) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    let pedidos = Pedidos::default();
    let registro = Arc::clone(&pedidos);
    thread::spawn(move || {
        let (mut cliente, _) = listener.accept().unwrap();
        let mut cabecalho = [0u8; 2];
        cliente.read_exact(&mut cabecalho).unwrap();
        let mut metodos = vec![0u8; usize::from(cabecalho[1])];
        cliente.read_exact(&mut metodos).unwrap();
        cliente.write_all(&[5, 0]).unwrap();
        let mut pedido = [0u8; 5];
        cliente.read_exact(&mut pedido).unwrap();
        assert_eq!(&pedido[..4], [5, 1, 0, 3]);
        let mut host = vec![0u8; usize::from(pedido[4])];
        cliente.read_exact(&mut host).unwrap();
        let mut porta = [0u8; 2];
        cliente.read_exact(&mut porta).unwrap();
        registro.lock().unwrap().push(format!(
            "{}:{}",
            String::from_utf8(host).unwrap(),
            u16::from_be_bytes(porta)
        ));
        let servidor = TcpStream::connect(("127.0.0.1", destino)).unwrap();
        cliente.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        ponte(cliente, servidor);
    });
    (porta, pedidos)
}

/// O mesmo `10.0.0.5:22` por dois proxies, cada um com o seu `known_hosts` (um por perfil de
/// rede): as duas abrem, cada proxy vê o mesmo destino e cada servidor conta um cliente.
#[test]
fn o_mesmo_ip_em_duas_redes_chega_a_servidores_diferentes() {
    let (a, b) = (Teste::novo("rede-a"), Teste::novo("rede-b"));
    let (proxy_a, pedidos_a) = proxy_socks5_fixo(a.porta);
    let (proxy_b, pedidos_b) = proxy_socks5_fixo(b.porta);
    let pasta = std::env::temp_dir().join(format!("tomni-ui-redes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pasta);
    std::fs::create_dir_all(&pasta).unwrap();
    let (conhecidos_a, conhecidos_b) = (pasta.join("a"), pasta.join("b"));
    learn_known_hosts_path("10.0.0.5", 22, &a.servidor, &conhecidos_a).unwrap();
    learn_known_hosts_path("10.0.0.5", 22, &b.servidor, &conhecidos_b).unwrap();

    let abrir = |proxy: u16, conhecidos: &Path| {
        Ssh::builder("10.0.0.5", 22, "ana")
            .known_hosts(conhecidos)
            .proxy(proxy_socks5_para(proxy, None, None))
            .password("senha")
            .connect()
    };
    let mut ssh_a = abrir(proxy_a, &conhecidos_a);
    let mut ssh_b = abrir(proxy_b, &conhecidos_b);
    assert_eq!(ler_ate(&mut ssh_a, "bem-vinda").1, Status::Open);
    assert_eq!(ler_ate(&mut ssh_b, "bem-vinda").1, Status::Open);
    assert_eq!(*pedidos_a.lock().unwrap(), ["10.0.0.5:22"]);
    assert_eq!(*pedidos_b.lock().unwrap(), ["10.0.0.5:22"]);
    assert_eq!((a.conexoes(), b.conexoes()), (1, 1));
}

/// Por que cada perfil tem o próprio `known_hosts`: num arquivo só, o mesmo `10.0.0.5:22` do
/// segundo cliente teria outra chave e a conexão seria recusada.
#[test]
fn o_mesmo_ip_com_um_known_hosts_so_acusa_a_chave_que_mudou() {
    let (a, b) = (Teste::novo("junto-a"), Teste::novo("junto-b"));
    let (proxy_a, _) = proxy_socks5_fixo(a.porta);
    let (proxy_b, _) = proxy_socks5_fixo(b.porta);
    let conhecidos = a.pasta.join("compartilhado");
    learn_known_hosts_path("10.0.0.5", 22, &a.servidor, &conhecidos).unwrap();
    let abrir = |proxy: u16| {
        Ssh::builder("10.0.0.5", 22, "ana")
            .known_hosts(&conhecidos)
            .proxy(proxy_socks5_para(proxy, None, None))
            .password("senha")
            .connect()
    };
    assert_eq!(ler_ate(&mut abrir(proxy_a), "bem-vinda").1, Status::Open);
    let (_, status) = ler_ate(&mut abrir(proxy_b), "bem-vinda");
    let Status::Closed(Some(mensagem)) = status else {
        panic!("devia recusar: {status:?}");
    };
    assert!(
        mensagem.starts_with("a chave do servidor mudou"),
        "{mensagem}"
    );
}

#[test]
fn before_connect_mostra_os_avisos_antes_da_saida_do_servidor() {
    let teste = Teste::novo("antes");
    teste.conhecer_servidor();
    let mut ssh = teste
        .builder()
        .password("senha")
        .before_connect(|aviso| {
            aviso("Subindo a rede Cliente A…");
            aviso("Rede Cliente A pronta.");
            Ok(())
        })
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
    let (subindo, pronta, boas_vindas) = (
        texto.find("Subindo a rede Cliente A…\r\n"),
        texto.find("Rede Cliente A pronta.\r\n"),
        texto.find("bem-vinda"),
    );
    assert!(
        subindo < pronta && pronta < boas_vindas && subindo.is_some(),
        "{texto:?}"
    );
}

#[test]
fn before_connect_com_erro_fecha_a_sessao_antes_do_proxy() {
    let teste = Teste::novo("antes-erro");
    teste.conhecer_servidor();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let porta_do_proxy = proxy.local_addr().unwrap().port();
    let mut ssh = teste
        .builder()
        .password("senha")
        .proxy(proxy_socks5_para(porta_do_proxy, None, None))
        .before_connect(|_| Err("a rede Cliente A não subiu: sem Docker".to_owned()))
        .connect();
    let (_, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(
        status,
        Status::Closed(Some("a rede Cliente A não subiu: sem Docker".into()))
    );
    let sem_conexao = proxy.accept().map(|_| ()).unwrap_err();
    assert_eq!(sem_conexao.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(teste.conexoes(), 0);
}

#[test]
fn before_connect_roda_de_novo_no_restart_e_na_duplicacao() {
    let teste = Teste::novo("antes-restart");
    teste.conhecer_servidor();
    let vezes = Arc::new(AtomicUsize::new(0));
    let contar = Arc::clone(&vezes);
    let mut ssh = teste
        .builder()
        .password("senha")
        .before_connect(move |_| {
            contar.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .connect();
    assert_eq!(ler_ate(&mut ssh, "bem-vinda").1, Status::Open);
    let mut copia = ssh.duplicate().unwrap();
    assert_eq!(
        ler_ate_transport(copia.as_mut(), "bem-vinda").1,
        Status::Open
    );
    assert_eq!(vezes.load(Ordering::SeqCst), 2);
}

#[test]
fn proxy_socks5_sem_autenticacao() {
    let teste = Teste::novo("socks-aberto");
    teste.conhecer_servidor();
    let (porta, pedidos) = proxy_socks5(None);
    let mut ssh = teste
        .builder()
        .proxy(proxy_socks5_para(porta, None, None))
        .password("senha")
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
    assert_eq!(
        *pedidos.lock().unwrap(),
        [format!("127.0.0.1:{}", teste.porta)]
    );
}

#[test]
fn proxy_socks5_com_usuario_e_senha() {
    let teste = Teste::novo("socks-senha");
    teste.conhecer_servidor();
    let (porta, _) = proxy_socks5(Some(("rui", "segredo")));
    let mut ssh = teste
        .builder()
        .proxy(proxy_socks5_para(porta, Some("rui"), Some("segredo")))
        .password("senha")
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
    ssh.write(b"pelo proxy").unwrap();
    let (texto, _) = ler_ate(&mut ssh, "pelo proxy");
    assert!(texto.ends_with("pelo proxy"), "{texto:?}");
}

#[test]
fn proxy_socks5_com_senha_errada() {
    let teste = Teste::novo("socks-errada");
    teste.conhecer_servidor();
    let (porta, _) = proxy_socks5(Some(("rui", "segredo")));
    let mut ssh = teste
        .builder()
        .proxy(proxy_socks5_para(porta, Some("rui"), Some("outra")))
        .password("senha")
        .connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert_eq!(
        status,
        Status::Closed(Some("o proxy recusou o usuário ou a senha".into()))
    );
    assert_eq!(teste.conexoes(), 0);
}

#[test]
fn proxy_http_sem_autenticacao() {
    let teste = Teste::novo("http-aberto");
    teste.conhecer_servidor();
    let (porta, pedidos) = proxy_http(None);
    let mut ssh = teste
        .builder()
        .proxy(proxy_http_para(porta, None, None))
        .password("senha")
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
    assert_eq!(
        *pedidos.lock().unwrap(),
        [format!("127.0.0.1:{}", teste.porta)]
    );
}

#[test]
fn proxy_http_com_usuario_e_senha() {
    let teste = Teste::novo("http-senha");
    teste.conhecer_servidor();
    let (porta, _) = proxy_http(Some(("rui", "segredo")));
    let mut ssh = teste
        .builder()
        .proxy(proxy_http_para(porta, Some("rui"), Some("segredo")))
        .password("senha")
        .connect();
    let (texto, status) = ler_ate(&mut ssh, "bem-vinda");
    assert_eq!(status, Status::Open, "{texto}");
}

#[test]
fn proxy_http_com_senha_errada_e_sem_credenciais() {
    let teste = Teste::novo("http-errada");
    teste.conhecer_servidor();
    let (porta, _) = proxy_http(Some(("rui", "segredo")));
    let mut ssh = teste
        .builder()
        .proxy(proxy_http_para(porta, Some("rui"), Some("outra")))
        .connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert_eq!(
        status,
        Status::Closed(Some("o proxy recusou o usuário ou a senha".into()))
    );

    let (porta, _) = proxy_http(Some(("rui", "segredo")));
    let mut ssh = teste
        .builder()
        .proxy(proxy_http_para(porta, None, None))
        .connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert_eq!(
        status,
        Status::Closed(Some("o proxy pediu autenticação".into()))
    );
}

#[test]
fn proxy_que_nao_existe() {
    let porta = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut ssh = Ssh::builder("127.0.0.1", 22, "ana")
        .proxy(proxy_http_para(porta, None, None))
        .connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert_eq!(
        status,
        Status::Closed(Some("proxy 127.0.0.1: conexão recusada".into()))
    );
}

// --- comando remoto (`exec`) e ssh-copy-id ---

#[cfg(unix)]
#[test]
fn exec_devolve_a_saida_o_erro_e_o_codigo() {
    let teste = Teste::novo("exec-saida");
    teste.conhecer_servidor();
    let saida = teste
        .builder()
        .password("senha")
        .exec("echo certo ; echo errado >&2 ; exit 4", b"")
        .unwrap();
    assert_eq!(saida.status, Some(4));
    assert!(!saida.success());
    assert_eq!(saida.stdout_text(), "certo\n");
    assert_eq!(saida.stderr_text(), "errado\n");
    // Sem terminal: nenhum `pty` nem `shell`.
    assert_eq!(
        teste.log(),
        ["exec echo certo ; echo errado >&2 ; exit 4".to_owned()]
    );
}

#[cfg(unix)]
#[test]
fn exec_manda_o_stdin_e_fecha_a_entrada() {
    let teste = Teste::novo("exec-stdin");
    teste.conhecer_servidor();
    // O `cat` só termina com o fim da entrada.
    let saida = teste
        .builder()
        .password("senha")
        .exec("cat", "uma linha\noutra\n".as_bytes())
        .unwrap();
    assert!(saida.success());
    assert_eq!(saida.stdout_text(), "uma linha\noutra\n");
}

#[test]
fn exec_com_senha_errada() {
    let teste = Teste::novo("exec-senha");
    teste.conhecer_servidor();
    let erro = teste
        .builder()
        .password("outra")
        .exec("true", b"")
        .unwrap_err();
    assert_eq!(erro, "autenticação recusada para ana");
    assert!(teste.log().is_empty());
}

#[cfg(unix)]
#[test]
fn exec_passa_pelos_saltos_e_cita_o_salto_que_falhou() {
    let (salto, destino) = (Teste::novo("exec-salto"), Teste::novo("exec-destino"));
    for t in [&salto, &destino] {
        t.conhecer_servidor();
    }
    let saida = destino
        .builder()
        .password("senha")
        .jump(salto.builder().password("senha"))
        .exec("echo oi", b"")
        .unwrap();
    assert_eq!(saida.stdout_text(), "oi\n");
    assert_eq!(salto.log(), [format!("túnel 127.0.0.1:{}", destino.porta)]);

    let erro = destino
        .builder()
        .password("senha")
        .jump(salto.builder().password("errada"))
        .exec("echo oi", b"")
        .unwrap_err();
    assert_eq!(
        erro,
        format!("127.0.0.1:{}: autenticação recusada para ana", salto.porta)
    );
}

#[cfg(unix)]
#[test]
fn exec_pergunta_a_chave_do_servidor_como_a_conexao() {
    let teste = Teste::novo("exec-chave");
    let vistas = Arc::new(Mutex::new(Vec::new()));
    let registro = Arc::clone(&vistas);
    let saida = teste
        .builder()
        .password("senha")
        .check_host_key(move |chave, conhecida| {
            registro.lock().unwrap().push(conhecida);
            chave.remember().is_ok()
        })
        .exec("echo oi", b"")
        .unwrap();
    assert_eq!(saida.stdout_text(), "oi\n");
    assert_eq!(*vistas.lock().unwrap(), [KnownHost::Unknown]);

    // Recusada, a conexão não chega a rodar nada.
    let outro = Teste::novo("exec-chave-recusada");
    let erro = outro
        .builder()
        .password("senha")
        .check_host_key(|_, _| false)
        .exec("echo oi", b"")
        .unwrap_err();
    assert!(!erro.is_empty());
    assert!(outro.log().is_empty());
}

/// A chave pública de teste, com comentário.
fn chave_publica(comentario: &str) -> PublicKey {
    let mut chave = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    chave.set_comment(comentario);
    chave.public_key().clone()
}

#[cfg(unix)]
fn authorized_keys(teste: &Teste) -> PathBuf {
    teste.home().join(".ssh").join("authorized_keys")
}

#[cfg(unix)]
#[test]
fn copy_id_cria_a_pasta_e_o_arquivo_com_as_permissoes_do_openssh() {
    use std::os::unix::fs::PermissionsExt;

    let teste = Teste::novo("copy-id-novo");
    teste.conhecer_servidor();
    let chave = chave_publica("ana@notebook");
    let resultado = copy_id(
        &teste.builder().password("senha"),
        &chave,
        CopyIdOptions::default(),
    )
    .unwrap();
    assert_eq!(
        resultado,
        CopyIdOutcome::Installed {
            path: ".ssh/authorized_keys".into()
        }
    );
    let arquivo = authorized_keys(&teste);
    let linha = keys::public_openssh(&chave);
    assert_eq!(
        std::fs::read_to_string(&arquivo).unwrap(),
        format!("{}\n", linha.trim_end())
    );
    let modo = |caminho: &Path| std::fs::metadata(caminho).unwrap().permissions().mode() & 0o777;
    assert_eq!(modo(&arquivo), 0o600);
    assert_eq!(modo(arquivo.parent().unwrap()), 0o700);
}

#[cfg(unix)]
#[test]
fn copy_id_junta_a_um_arquivo_sem_quebra_de_linha_no_fim() {
    let teste = Teste::novo("copy-id-junta");
    teste.conhecer_servidor();
    std::fs::create_dir_all(teste.home().join(".ssh")).unwrap();
    std::fs::write(authorized_keys(&teste), "ssh-rsa AAAAvelho antiga").unwrap();
    let chave = chave_publica("ana@notebook");
    copy_id(
        &teste.builder().password("senha"),
        &chave,
        CopyIdOptions::default(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(authorized_keys(&teste)).unwrap(),
        format!(
            "ssh-rsa AAAAvelho antiga\n{}\n",
            keys::public_openssh(&chave).trim_end()
        )
    );
}

#[cfg(unix)]
#[test]
fn copy_id_nao_repete_a_chave_que_ja_esta_la_a_nao_ser_com_force() {
    let teste = Teste::novo("copy-id-repete");
    teste.conhecer_servidor();
    let chave = chave_publica("ana@notebook");
    let builder = teste.builder().password("senha");
    let instalar = |options| copy_id(&builder, &chave, options).unwrap();

    assert!(matches!(
        instalar(CopyIdOptions::default()),
        CopyIdOutcome::Installed { .. }
    ));
    let depois_da_primeira = std::fs::read_to_string(authorized_keys(&teste)).unwrap();
    assert_eq!(
        instalar(CopyIdOptions::default()),
        CopyIdOutcome::AlreadyThere
    );
    assert_eq!(
        std::fs::read_to_string(authorized_keys(&teste)).unwrap(),
        depois_da_primeira
    );

    // Com `force`, instala de novo.
    let forcada = CopyIdOptions {
        force: true,
        dry_run: false,
    };
    assert!(matches!(instalar(forcada), CopyIdOutcome::Installed { .. }));
    assert_eq!(
        std::fs::read_to_string(authorized_keys(&teste)).unwrap(),
        format!("{depois_da_primeira}{depois_da_primeira}")
    );
}

#[cfg(unix)]
#[test]
fn copy_id_reconhece_a_chave_com_opcoes_e_outro_comentario() {
    let teste = Teste::novo("copy-id-opcoes");
    teste.conhecer_servidor();
    let chave = chave_publica("ana@notebook");
    let linha = keys::public_openssh(&chave);
    let mut partes = linha.split_whitespace();
    let (tipo, blob) = (partes.next().unwrap(), partes.next().unwrap());
    std::fs::create_dir_all(teste.home().join(".ssh")).unwrap();
    std::fs::write(
        authorized_keys(&teste),
        format!(
            "# do trabalho\nfrom=\"10.0.0.0/8\",command=\"ls\" {tipo} {blob} outro comentário\n"
        ),
    )
    .unwrap();
    let resultado = copy_id(
        &teste.builder().password("senha"),
        &chave,
        CopyIdOptions::default(),
    )
    .unwrap();
    assert_eq!(resultado, CopyIdOutcome::AlreadyThere);
}

#[cfg(unix)]
#[test]
fn copy_id_com_dry_run_nao_muda_nada() {
    let teste = Teste::novo("copy-id-dry");
    teste.conhecer_servidor();
    let chave = chave_publica("ana@notebook");
    let resultado = copy_id(
        &teste.builder().password("senha"),
        &chave,
        CopyIdOptions {
            force: false,
            dry_run: true,
        },
    )
    .unwrap();
    assert_eq!(
        resultado,
        CopyIdOutcome::WouldInstall {
            path: ".ssh/authorized_keys".into()
        }
    );
    assert!(!teste.home().join(".ssh").exists());
}

#[test]
fn copy_id_com_senha_errada_nao_instala() {
    let teste = Teste::novo("copy-id-senha");
    teste.conhecer_servidor();
    let erro = copy_id(
        &teste.builder().password("outra"),
        &chave_publica("ana@notebook"),
        CopyIdOptions::default(),
    )
    .unwrap_err();
    assert_eq!(erro, "autenticação recusada para ana");
}

/// Os comandos que o servidor Windows simulado recebeu, sem os scripts.
fn comandos_do_windows(teste: &Teste) -> Vec<String> {
    teste
        .log()
        .into_iter()
        .filter_map(|linha| linha.strip_prefix("exec ").map(str::to_owned))
        .collect()
}

#[test]
fn copy_id_no_windows_de_administrador_usa_o_arquivo_dos_administradores() {
    let teste = Teste::windows("copy-id-win-admin", true);
    teste.conhecer_servidor();
    let chave = chave_publica("ana@notebook");
    let linha = keys::public_openssh(&chave);
    let builder = teste.builder().password("senha");
    let caminho = "C:\\ProgramData\\ssh\\administrators_authorized_keys";

    let resultado = copy_id(&builder, &chave, CopyIdOptions::default()).unwrap();
    assert_eq!(
        resultado,
        CopyIdOutcome::Installed {
            path: caminho.into()
        }
    );
    assert_eq!(
        *teste.modo.arquivo.lock().unwrap(),
        format!("{}\r\n", linha.trim_end())
    );

    // O sistema foi descoberto pelo `uname` que falhou e pelo PowerShell; a chave foi pelo
    // stdin, nunca no texto de um comando; e o arquivo dos administradores leva o `icacls`.
    let log = teste.log();
    let comandos = comandos_do_windows(&teste);
    assert_eq!(comandos[0], "echo $0 ; uname -s");
    assert!(comandos[1].contains("OSVersion.Platform"));
    assert!(comandos.iter().all(|c| !c.contains(linha.trim_end())));
    assert!(
        log.iter().any(|l| l.starts_with("script ")
            && l.contains("icacls")
            && l.contains("$admin = $true"))
    );
    assert!(log.contains(&format!("stdin {}", linha.trim_end())));

    // Segunda vez: já está lá.
    let resultado = copy_id(&builder, &chave, CopyIdOptions::default()).unwrap();
    assert_eq!(resultado, CopyIdOutcome::AlreadyThere);
    assert_eq!(
        teste
            .modo
            .arquivo
            .lock()
            .unwrap()
            .matches("ssh-ed25519")
            .count(),
        1
    );

    // Com `dry_run` outra chave só seria instalada.
    let outra = chave_publica("ana@desktop");
    let resultado = copy_id(
        &builder,
        &outra,
        CopyIdOptions {
            force: false,
            dry_run: true,
        },
    )
    .unwrap();
    assert_eq!(
        resultado,
        CopyIdOutcome::WouldInstall {
            path: caminho.into()
        }
    );
    assert_eq!(
        teste
            .modo
            .arquivo
            .lock()
            .unwrap()
            .matches("ssh-ed25519")
            .count(),
        1
    );
}

#[test]
fn copy_id_no_windows_de_usuario_comum_usa_o_authorized_keys_do_perfil() {
    let teste = Teste::windows("copy-id-win-user", false);
    teste.conhecer_servidor();
    let chave = chave_publica("ana@notebook");
    let resultado = copy_id(
        &teste.builder().password("senha"),
        &chave,
        CopyIdOptions::default(),
    )
    .unwrap();
    assert_eq!(
        resultado,
        CopyIdOutcome::Installed {
            path: "C:\\Users\\ana\\.ssh\\authorized_keys".into()
        }
    );
    // Sem `icacls`: o arquivo do perfil herda as permissões da pasta.
    assert!(teste.log().iter().any(|l| l.contains("$admin = $false")));
}

mod tuneis;

// --- `SshChannelOpener::exec` e a compressão ---

/// Dá `restante` bytes de `0xAB` e, depois, um erro (como o cancelamento de um envio).
#[cfg(unix)]
struct EntradaQueFalha {
    restante: usize,
}

#[cfg(unix)]
impl Read for EntradaQueFalha {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.restante == 0 {
            return Err(io::Error::other("cancelado"));
        }
        let n = buf.len().min(self.restante);
        buf[..n].fill(0xAB);
        self.restante -= n;
        Ok(n)
    }
}

#[cfg(unix)]
#[test]
fn o_opener_manda_100_kib_pelo_exec_e_conta_o_enviado() {
    let teste = Teste::novo("opener-exec");
    teste.conhecer_servidor();
    let ssh = teste.builder().password("senha").connect();
    let canais = ssh.ssh_channels().expect("uma sessão SSH entrega o opener");
    let dados: Vec<u8> = (0..102_400u32).map(|i| (i % 251) as u8).collect();
    let mut enviados = Vec::new();
    let saida = canais
        .exec(
            "cat > arquivo",
            &mut dados.as_slice(),
            &mut |total| enviados.push(total),
            Duration::from_secs(10),
        )
        .unwrap();
    assert!(saida.success(), "{saida:?}");
    assert_eq!(std::fs::read(teste.home().join("arquivo")).unwrap(), dados);
    assert_eq!(enviados.last(), Some(&102_400));
    assert!(
        enviados.windows(2).all(|par| par[0] < par[1]),
        "{enviados:?}"
    );
}

#[cfg(unix)]
#[test]
fn o_opener_devolve_o_codigo_e_o_stderr_do_comando() {
    let teste = Teste::novo("opener-exec-codigo");
    teste.conhecer_servidor();
    let ssh = teste.builder().password("senha").connect();
    let canais = ssh.ssh_channels().unwrap();
    let saida = canais
        .exec(
            "echo um ; echo dois >&2 ; exit 3",
            &mut std::io::empty(),
            &mut |_| {},
            Duration::from_secs(10),
        )
        .unwrap();
    assert_eq!(saida.status, Some(3));
    assert_eq!(saida.stdout_text(), "um\n");
    assert_eq!(saida.stderr_text(), "dois\n");
}

#[cfg(unix)]
#[test]
fn entrada_que_da_erro_fecha_o_canal_sem_eof_e_devolve_o_erro() {
    let teste = Teste::novo("opener-exec-cancela");
    teste.conhecer_servidor();
    let ssh = teste.builder().password("senha").connect();
    let canais = ssh.ssh_channels().unwrap();
    let erro = canais
        .exec(
            "cat > pela-metade",
            &mut EntradaQueFalha {
                restante: 64 * 1024,
            },
            &mut |_| {},
            Duration::from_secs(10),
        )
        .unwrap_err();
    assert_eq!(erro.to_string(), "cancelado");
    esperar(|| teste.log().iter().any(|l| l.starts_with("fechou-sem-eof")));
    let log = teste.log();
    assert!(
        log.contains(&"fechou-sem-eof cat > pela-metade (65536 bytes)".to_owned()),
        "{log:?}"
    );
    // O comando nunca rodou: não houve EOF.
    assert!(!log.iter().any(|l| l.starts_with("exec ")), "{log:?}");
    assert!(!teste.home().join("pela-metade").exists());
}

#[cfg(unix)]
#[test]
fn o_exec_do_opener_numa_sessao_fechada_da_erro() {
    let teste = Teste::novo("opener-exec-fechada");
    teste.conhecer_servidor();
    let ssh = teste.builder().password("senha").connect();
    let canais = ssh.ssh_channels().unwrap();
    drop(ssh);
    thread::sleep(Duration::from_millis(200));
    let erro = canais
        .exec(
            "true",
            &mut std::io::empty(),
            &mut |_| {},
            Duration::from_secs(5),
        )
        .unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::ConnectionReset);
}

#[cfg(unix)]
#[test]
fn shell_dir_roda_a_sonda_e_nao_acha_o_sshd_do_servidor_de_teste() {
    let teste = Teste::novo("opener-shell-dir");
    teste.conhecer_servidor();
    let ssh = teste.builder().password("senha").connect();
    let canais = ssh.ssh_channels().unwrap();
    // O servidor de teste roda o comando num `sh` local: não há `sshd` entre os antepassados
    // (a menos que a suíte rode dentro de uma sessão SSH, e então a pasta é uma pasta absoluta).
    let pasta = canais.shell_dir(Duration::from_secs(10)).unwrap();
    assert!(
        pasta.as_deref().is_none_or(|p| p.starts_with('/')),
        "{pasta:?}"
    );
    let log = teste.log();
    assert!(
        log.iter()
            .any(|l| l.starts_with("exec sh -c '") && l.contains("/proc/$1/stat")),
        "{log:?}"
    );
}

#[cfg(unix)]
#[test]
fn shell_dir_numa_sessao_fechada_da_erro() {
    let teste = Teste::novo("opener-shell-dir-fechada");
    teste.conhecer_servidor();
    let ssh = teste.builder().password("senha").connect();
    let canais = ssh.ssh_channels().unwrap();
    drop(ssh);
    thread::sleep(Duration::from_millis(200));
    let erro = canais.shell_dir(Duration::from_secs(5)).unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::ConnectionReset);
}

/// Um servidor de eco local: devolve cada conexão byte a byte até ela fechar.
fn servidor_de_eco() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for conexao in listener.incoming().flatten() {
            thread::spawn(move || {
                let (mut leitura, mut escrita) = (conexao.try_clone().unwrap(), conexao);
                let _ = std::io::copy(&mut leitura, &mut escrita);
            });
        }
    });
    porta
}

fn ecoar_pelo_canal(canal: SshChannel, texto: &[u8]) -> Vec<u8> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let mut canal = canal;
        canal.write_all(texto).await.unwrap();
        let mut volta = vec![0u8; texto.len()];
        tokio::time::timeout(Duration::from_secs(5), canal.read_exact(&mut volta))
            .await
            .expect("o eco devia voltar")
            .unwrap();
        canal.shutdown().await.unwrap();
        volta
    })
}

#[test]
fn open_direct_leva_os_bytes_ate_o_destino_e_traz_a_volta() {
    let eco = servidor_de_eco();
    let teste = Teste::novo("opener-direct");
    teste.conhecer_servidor();
    let ssh = teste.builder().password("senha").connect();
    let canais = ssh.ssh_channels().unwrap();
    let canal = canais
        .open_direct("127.0.0.1", eco, Duration::from_secs(5))
        .unwrap();
    assert_eq!(ecoar_pelo_canal(canal, b"ola, servidor"), b"ola, servidor");
    assert!(
        teste.log().contains(&format!("túnel 127.0.0.1:{eco}")),
        "{:?}",
        teste.log()
    );
}

#[test]
fn o_canal_direto_mantem_a_conexao_depois_do_exit_do_shell() {
    let eco = servidor_de_eco();
    let teste = Teste::novo("opener-direct-exit");
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("senha").connect();
    ler_ate(&mut ssh, "bem-vinda, ana\r\n");
    let canais = ssh.ssh_channels().unwrap();
    let canal = canais
        .open_direct("127.0.0.1", eco, Duration::from_secs(5))
        .unwrap();
    ssh.write(b"exit 3\r").unwrap();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert!(matches!(status, Status::Closed(Some(_))), "{status:?}");
    assert_eq!(ecoar_pelo_canal(canal, b"ainda aqui"), b"ainda aqui");
}

#[test]
fn open_direct_para_um_destino_fechado_diz_que_nada_escuta() {
    let fechada = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let teste = Teste::novo("opener-direct-fechada");
    teste.conhecer_servidor();
    let ssh = teste.builder().password("senha").connect();
    let canais = ssh.ssh_channels().unwrap();
    let erro = canais
        .open_direct("127.0.0.1", fechada, Duration::from_secs(5))
        .unwrap_err();
    assert_eq!(
        erro.to_string(),
        format!("Nada escuta em 127.0.0.1:{fechada} no servidor.")
    );
}

#[test]
fn open_direct_com_o_servidor_que_proibe_tuneis_traduz_a_recusa() {
    let teste = Teste::novo("opener-direct-proibido");
    teste.modo.bloqueia_tuneis.store(true, Ordering::SeqCst);
    teste.conhecer_servidor();
    let ssh = teste.builder().password("senha").connect();
    let canais = ssh.ssh_channels().unwrap();
    let erro = canais
        .open_direct("127.0.0.1", 80, Duration::from_secs(5))
        .unwrap_err();
    assert!(erro.to_string().contains("AllowTcpForwarding"), "{erro}");
}

#[test]
fn open_direct_numa_sessao_fechada_da_erro() {
    let teste = Teste::novo("opener-direct-sessao-fechada");
    teste.conhecer_servidor();
    let ssh = teste.builder().password("senha").connect();
    let canais = ssh.ssh_channels().unwrap();
    drop(ssh);
    thread::sleep(Duration::from_millis(200));
    let erro = canais
        .open_direct("127.0.0.1", 80, Duration::from_secs(5))
        .unwrap_err();
    assert_eq!(erro.kind(), io::ErrorKind::ConnectionReset);
}

#[test]
fn a_compressao_pede_zlib_so_no_destino() {
    let destino = Ssh::builder("destino", 22, "ana").compression(true);
    let config = client_config(&destino, true);
    assert_eq!(
        config.preferred.compression.as_ref(),
        [
            russh::compression::ZLIB_LEGACY,
            russh::compression::ZLIB,
            russh::compression::NONE
        ]
    );
    // Num salto, a conexão já vai cifrada dentro de outra: o padrão do russh.
    let salto = client_config(&destino, false);
    assert_eq!(
        salto.preferred.compression,
        russh::Preferred::default().compression
    );
    // Desligada (o padrão), nada muda.
    let sem = client_config(&Ssh::builder("destino", 22, "ana"), true);
    assert_eq!(
        sem.preferred.compression,
        russh::Preferred::default().compression
    );
    assert!(format!("{destino:?}").contains("compression: true"));
}

#[cfg(unix)]
#[test]
fn com_compressao_a_sessao_abre_e_os_100_kib_chegam_inteiros() {
    for (nome, so_none) in [("compressao-zlib", false), ("compressao-so-none", true)] {
        let teste = Teste::novo(nome);
        teste.modo.sem_compressao.store(so_none, Ordering::SeqCst);
        teste.conhecer_servidor();
        let mut ssh = teste
            .builder()
            .password("senha")
            .compression(true)
            .connect();
        let (texto, status) = ler_ate(&mut ssh, "bem-vinda, ana\r\n");
        assert_eq!(status, Status::Open, "{nome}: {texto}");
        let canais = ssh.ssh_channels().unwrap();
        let dados = vec![b'z'; 102_400];
        let saida = canais
            .exec(
                "cat > arquivo",
                &mut dados.as_slice(),
                &mut |_| {},
                Duration::from_secs(10),
            )
            .unwrap();
        assert!(saida.success(), "{nome}: {saida:?}");
        assert_eq!(
            std::fs::read(teste.home().join("arquivo")).unwrap(),
            dados,
            "{nome}"
        );
    }
}

#[test]
fn conectando_ate_o_shell_abrir_e_de_novo_no_reopen() {
    let teste = Teste::novo("conectando");
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("senha").connect();
    assert!(ssh.connecting(), "recém criada, a conexão está abrindo");
    ler_ate(&mut ssh, "bem-vinda, ana\r\n");
    esperar(|| !ssh.connecting());
    assert!(!ssh.connecting(), "com o shell aberto, deixa de conectar");

    ssh.write(b"exit 3\r").unwrap();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert!(matches!(status, Status::Closed(Some(_))), "{status:?}");
    assert!(ssh.reopen());
    assert!(ssh.connecting(), "reabrir volta a conectar");
    ler_ate(&mut ssh, "bem-vinda, ana\r\n");
    esperar(|| !ssh.connecting());
    assert!(!ssh.connecting());
}

#[test]
fn falha_antes_do_shell_deixa_de_conectar() {
    let teste = Teste::novo("conectando-falha");
    teste.conhecer_servidor();
    let mut ssh = teste.builder().password("outra").connect();
    let (_, status) = ler_ate(&mut ssh, "\u{0}");
    assert!(matches!(status, Status::Closed(Some(_))), "{status:?}");
    esperar(|| !ssh.connecting());
    assert!(!ssh.connecting());
}
