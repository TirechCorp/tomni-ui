//! Telnet sobre TCP (RFC 854), com as negociações comuns: ECHO e SGA do servidor
//! (o modo "um caractere por vez"), NAWS para o tamanho da janela (RFC 1073) e TTYPE para o
//! tipo de terminal (RFC 1091).

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use tomni_ui_net::{CONNECT_TIMEOUT, Proxy, connect_error_message, connect_tcp};

use crate::transport::{Inbox, Status, Transport, closed_error, error_message};

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

const ECHO: u8 = 1;
const SGA: u8 = 3;
const TTYPE: u8 = 24;
const NAWS: u8 = 31;

const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;

/// Tipo de terminal mandado no TTYPE, o mesmo do `TERM` do [`Process`](crate::Process).
const TERMINAL_TYPE: &[u8] = b"XTERM-256COLOR";

/// Uma conexão Telnet, aberta numa thread: o terminal mostra o que chega assim que conecta, e
/// um erro ("conexão recusada") aparece no fim da tela.
///
/// Negocia assim: pede ao servidor o eco e a supressão do "go ahead" (ECHO e SGA),
/// oferece o tamanho da janela (NAWS, mandado de novo a cada redimensionamento) e responde
/// `XTERM-256COLOR` ao tipo de terminal (TTYPE). O Enter vai como CR LF, a "nova linha" do
/// Telnet.
///
/// ```no_run
/// use tomni_ui_terminal::{Telnet, Terminal};
///
/// let console = Terminal::new(Telnet::connect("towel.blinkenlights.nl", 23)).id("console");
/// ```
#[derive(Debug)]
pub struct Telnet {
    inbox: Inbox,
    commands: Sender<Command>,
    /// Como a conexão foi pedida, para reabrir e duplicar.
    builder: TelnetBuilder,
    /// Verdadeiro até o TCP conectar; uma flag nova a cada `reopen`.
    connecting: Arc<AtomicBool>,
}

/// O gancho de [`TelnetBuilder::before_connect`].
type BeforeConnect = Arc<Mutex<dyn FnMut(&mut dyn FnMut(&str)) -> Result<(), String> + Send>>;

/// O servidor de um [`Telnet`] e por onde chegar a ele, antes de conectar.
#[derive(Clone)]
pub struct TelnetBuilder {
    host: String,
    port: u16,
    proxy: Option<Proxy>,
    before: Option<BeforeConnect>,
}

impl fmt::Debug for TelnetBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelnetBuilder")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("proxy", &self.proxy)
            .field("before_connect", &self.before.as_ref().map(|_| "…"))
            .finish()
    }
}

impl TelnetBuilder {
    /// Passa a conexão TCP por um proxy SOCKS5 ou HTTP CONNECT. O nome do servidor vai ao
    /// proxy sem ser resolvido aqui: quem resolve é o proxy, na rede dele.
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// Roda `f` na thread da conexão, antes de abrir o TCP (e antes do proxy). As linhas
    /// passadas a `aviso` aparecem no terminal; um `Err` fecha a sessão com a mensagem, como
    /// um erro de conexão. Roda de novo a cada reconexão, como o `SshBuilder::before_connect`
    /// (feature `ssh`).
    pub fn before_connect(
        mut self,
        f: impl FnMut(&mut dyn FnMut(&str)) -> Result<(), String> + Send + 'static,
    ) -> Self {
        self.before = Some(Arc::new(Mutex::new(f)));
        self
    }

    /// Conecta, sem bloquear.
    pub fn connect(self) -> Telnet {
        let inbox = Inbox::new();
        let (commands, connecting) = start(&self, &inbox);
        Telnet {
            inbox,
            commands,
            builder: self,
            connecting,
        }
    }
}

#[derive(Debug)]
enum Command {
    /// Bytes do terminal, ainda sem o escape do IAC.
    Data(Vec<u8>),
    Resize(u16, u16),
    /// Um pedido do servidor, lido pela thread de leitura.
    Received(Event),
    Close,
}

impl Telnet {
    /// Conecta a `host` na `port` (23 é a padrão do Telnet), sem bloquear.
    pub fn connect(host: impl Into<String>, port: u16) -> Self {
        Self::builder(host, port).connect()
    }

    /// `host` na `port`, para escolher o proxy e o gancho antes de conectar.
    ///
    /// ```no_run
    /// use tomni_ui_terminal::{Proxy, Telnet};
    ///
    /// let telnet = Telnet::builder("10.0.0.5", 23)
    ///     .proxy(Proxy::Socks5 {
    ///         host: "127.0.0.1".into(),
    ///         port: 1081,
    ///         user: None,
    ///         password: None,
    ///     })
    ///     .connect();
    /// ```
    pub fn builder(host: impl Into<String>, port: u16) -> TelnetBuilder {
        TelnetBuilder {
            host: host.into(),
            port,
            proxy: None,
            before: None,
        }
    }
}

/// Abre a thread da conexão e devolve por onde mandar os comandos a ela.
fn start(builder: &TelnetBuilder, inbox: &Inbox) -> (Sender<Command>, Arc<AtomicBool>) {
    let (commands, receiver) = mpsc::channel();
    let builder = builder.clone();
    let (thread_inbox, replies) = (inbox.clone(), commands.clone());
    let connecting = Arc::new(AtomicBool::new(true));
    let thread_connecting = Arc::clone(&connecting);
    let spawned = thread::Builder::new()
        .name("tomni-ui-telnet".into())
        .spawn(move || {
            let _mark = thread_inbox.mark_thread();
            run(
                &builder,
                &receiver,
                &replies,
                &thread_inbox,
                &thread_connecting,
            );
            ready(&thread_connecting);
        });
    if let Err(error) = spawned {
        inbox.close(Some(error_message(&error)));
        connecting.store(false, Ordering::Release);
    }
    (commands, connecting)
}

/// A conexão ficou pronta (ou acabou): acorda a interface para a aba perder o anel.
fn ready(connecting: &AtomicBool) {
    if connecting.swap(false, Ordering::AcqRel) {
        tomni_ui_core::wake();
    }
}

impl Drop for Telnet {
    fn drop(&mut self) {
        // A thread de leitura parada esperando vaga na fila segue e acaba com a conexão.
        self.inbox.detach();
        let _ = self.commands.send(Command::Close);
    }
}

impl Transport for Telnet {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.commands
            .send(Command::Data(bytes.to_vec()))
            .map_err(|_| closed_error())
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self.commands.send(Command::Resize(cols, rows));
    }

    fn read(&mut self, buf: &mut Vec<u8>, max: usize) -> Status {
        self.inbox.read(buf, max)
    }

    fn pending(&self) -> bool {
        self.inbox.pending()
    }

    fn threads(&self) -> Vec<u32> {
        self.inbox.threads()
    }

    fn connecting(&self) -> bool {
        self.connecting.load(Ordering::Acquire)
    }

    fn reopen(&mut self) -> bool {
        // A thread antiga já terminou (a fila fechou); o `Close` só garante.
        let _ = self.commands.send(Command::Close);
        self.inbox.reopen();
        (self.commands, self.connecting) = start(&self.builder, &self.inbox);
        true
    }

    fn duplicate(&self) -> Option<Box<dyn Transport>> {
        Some(Box::new(self.builder.clone().connect()))
    }
}

/// A thread da conexão: conecta, solta a leitura numa segunda thread e fica escrevendo o que
/// o terminal manda e as respostas das negociações.
fn run(
    builder: &TelnetBuilder,
    commands: &Receiver<Command>,
    replies: &Sender<Command>,
    inbox: &Inbox,
    connecting: &AtomicBool,
) {
    let fail = |error: &io::Error| inbox.close(Some(connect_error_message(error)));
    if let Some(before) = &builder.before {
        let mut before = before.lock().unwrap_or_else(PoisonError::into_inner);
        let mut notice = |line: &str| inbox.push(format!("{line}\r\n").as_bytes());
        if let Err(message) = before(&mut notice) {
            return inbox.close(Some(message));
        }
    }
    let opened = match &builder.proxy {
        Some(proxy) => proxy.open(&builder.host, builder.port, CONNECT_TIMEOUT),
        None => connect_tcp(&builder.host, builder.port, CONNECT_TIMEOUT)
            .map_err(|error| connect_error_message(&error)),
    };
    let mut stream = match opened {
        Ok(stream) => stream,
        Err(message) => return inbox.close(Some(message)),
    };
    ready(connecting);
    let _ = stream.set_nodelay(true);
    let reader = match stream.try_clone() {
        Ok(reader) => reader,
        Err(error) => return fail(&error),
    };
    let (read_inbox, read_replies) = (inbox.clone(), replies.clone());
    if let Err(error) = thread::Builder::new()
        .name("tomni-ui-telnet-leitura".into())
        .spawn(move || {
            let _mark = read_inbox.mark_thread();
            read_loop(reader, &read_inbox, &read_replies);
        })
    {
        return fail(&error);
    }

    let mut negotiation = Negotiation::default();
    let mut out = Vec::new();
    negotiation.start(&mut out);
    if let Err(error) = stream.write_all(&out) {
        return fail(&error);
    }
    for command in commands {
        out.clear();
        match command {
            Command::Data(bytes) => encode(&bytes, &mut out),
            Command::Resize(cols, rows) => negotiation.resize(cols, rows, &mut out),
            Command::Received(event) => negotiation.receive(event, &mut out),
            Command::Close => break,
        }
        if let Err(error) = stream.write_all(&out) {
            inbox.close(Some(error_message(&error)));
            break;
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

fn read_loop(mut reader: TcpStream, inbox: &Inbox, replies: &Sender<Command>) {
    let mut decoder = Decoder::default();
    let mut buf = vec![0; 16 * 1024];
    let (mut data, mut events) = (Vec::new(), Vec::new());
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return inbox.close(None),
            Ok(n) => {
                decoder.decode(&buf[..n], &mut data, &mut events);
                inbox.push_wait(&data);
                data.clear();
                for event in events.drain(..) {
                    let _ = replies.send(Command::Received(event));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return inbox.close(Some(error_message(&error))),
        }
    }
}

/// O que o servidor pediu: `IAC verbo opção` ou uma subnegociação.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Verb(u8, u8),
    Sub(u8, Vec<u8>),
}

#[derive(Debug, Default)]
enum DecodeState {
    #[default]
    Data,
    /// Depois de um CR: um NUL que vem em seguida é descartado.
    Cr,
    Iac,
    Verb(u8),
    /// Dentro de `IAC SB opção ...`.
    Sub(u8, Vec<u8>),
    SubIac(u8, Vec<u8>),
}

/// Separa os dados dos comandos do Telnet que chegam do servidor.
#[derive(Debug, Default)]
struct Decoder {
    state: DecodeState,
}

impl Decoder {
    fn decode(&mut self, input: &[u8], data: &mut Vec<u8>, events: &mut Vec<Event>) {
        for &byte in input {
            self.state = match std::mem::take(&mut self.state) {
                DecodeState::Data | DecodeState::Cr if byte == IAC => DecodeState::Iac,
                DecodeState::Cr if byte == 0 => DecodeState::Data,
                DecodeState::Data | DecodeState::Cr => {
                    data.push(byte);
                    if byte == b'\r' {
                        DecodeState::Cr
                    } else {
                        DecodeState::Data
                    }
                }
                DecodeState::Iac => match byte {
                    IAC => {
                        data.push(IAC);
                        DecodeState::Data
                    }
                    WILL | WONT | DO | DONT => DecodeState::Verb(byte),
                    SB => DecodeState::Verb(SB),
                    // NOP, GA, DM e os outros comandos de um byte.
                    _ => DecodeState::Data,
                },
                DecodeState::Verb(SB) => DecodeState::Sub(byte, Vec::new()),
                DecodeState::Verb(verb) => {
                    events.push(Event::Verb(verb, byte));
                    DecodeState::Data
                }
                DecodeState::Sub(option, mut sub) => {
                    if byte == IAC {
                        DecodeState::SubIac(option, sub)
                    } else {
                        sub.push(byte);
                        DecodeState::Sub(option, sub)
                    }
                }
                DecodeState::SubIac(option, mut sub) => match byte {
                    SE => {
                        events.push(Event::Sub(option, sub));
                        DecodeState::Data
                    }
                    _ => {
                        sub.push(byte);
                        DecodeState::Sub(option, sub)
                    }
                },
            };
        }
    }
}

/// Bytes do terminal para a rede: o IAC dobrado e o Enter (CR) como CR LF.
fn encode(bytes: &[u8], out: &mut Vec<u8>) {
    for (i, &byte) in bytes.iter().enumerate() {
        match byte {
            IAC => out.extend_from_slice(&[IAC, IAC]),
            b'\r' if bytes.get(i + 1) != Some(&b'\n') => out.extend_from_slice(b"\r\n"),
            _ => out.push(byte),
        }
    }
}

/// Estado das opções, dos dois lados, com o "pedido em andamento" da RFC 1143 para não entrar
/// em laço com um servidor que repete tudo (um eco).
#[derive(Debug)]
struct Negotiation {
    /// Opções que o terminal faz (NAWS, TTYPE).
    us: [bool; 256],
    us_asked: [bool; 256],
    /// Opções que o servidor faz (ECHO, SGA).
    him: [bool; 256],
    him_asked: [bool; 256],
    size: Option<(u16, u16)>,
}

impl Default for Negotiation {
    fn default() -> Self {
        Self {
            us: [false; 256],
            us_asked: [false; 256],
            him: [false; 256],
            him_asked: [false; 256],
            size: None,
        }
    }
}

impl Negotiation {
    /// O que o cliente manda ao conectar.
    fn start(&mut self, out: &mut Vec<u8>) {
        for option in [NAWS, TTYPE] {
            self.us_asked[usize::from(option)] = true;
            out.extend_from_slice(&[IAC, WILL, option]);
        }
        for option in [ECHO, SGA] {
            self.him_asked[usize::from(option)] = true;
            out.extend_from_slice(&[IAC, DO, option]);
        }
    }

    fn receive(&mut self, event: Event, out: &mut Vec<u8>) {
        match event {
            Event::Verb(verb, option) => self.verb(verb, option, out),
            Event::Sub(TTYPE, sub) if sub.first() == Some(&TTYPE_SEND) => {
                if self.us[usize::from(TTYPE)] {
                    out.extend_from_slice(&[IAC, SB, TTYPE, TTYPE_IS]);
                    out.extend_from_slice(TERMINAL_TYPE);
                    out.extend_from_slice(&[IAC, SE]);
                }
            }
            Event::Sub(..) => {}
        }
    }

    fn verb(&mut self, verb: u8, option: u8, out: &mut Vec<u8>) {
        let i = usize::from(option);
        match verb {
            WILL if matches!(option, ECHO | SGA) => {
                if !self.him[i] {
                    self.him[i] = true;
                    if !std::mem::take(&mut self.him_asked[i]) {
                        out.extend_from_slice(&[IAC, DO, option]);
                    }
                }
            }
            WILL => out.extend_from_slice(&[IAC, DONT, option]),
            WONT => {
                if self.him[i] || self.him_asked[i] {
                    self.him[i] = false;
                    if !std::mem::take(&mut self.him_asked[i]) {
                        out.extend_from_slice(&[IAC, DONT, option]);
                    }
                }
            }
            DO if matches!(option, NAWS | TTYPE | SGA) => {
                if !self.us[i] {
                    self.us[i] = true;
                    if !std::mem::take(&mut self.us_asked[i]) {
                        out.extend_from_slice(&[IAC, WILL, option]);
                    }
                    if option == NAWS {
                        self.send_size(out);
                    }
                }
            }
            DO => out.extend_from_slice(&[IAC, WONT, option]),
            DONT if self.us[i] || self.us_asked[i] => {
                self.us[i] = false;
                if !std::mem::take(&mut self.us_asked[i]) {
                    out.extend_from_slice(&[IAC, WONT, option]);
                }
            }
            _ => {}
        }
    }

    fn resize(&mut self, cols: u16, rows: u16, out: &mut Vec<u8>) {
        self.size = Some((cols, rows));
        if self.us[usize::from(NAWS)] {
            self.send_size(out);
        }
    }

    /// `IAC SB NAWS largura altura IAC SE`, com 16 bits cada e o IAC dobrado.
    fn send_size(&self, out: &mut Vec<u8>) {
        let Some((cols, rows)) = self.size else {
            return;
        };
        out.extend_from_slice(&[IAC, SB, NAWS]);
        for byte in cols.to_be_bytes().into_iter().chain(rows.to_be_bytes()) {
            out.push(byte);
            if byte == IAC {
                out.push(IAC);
            }
        }
        out.extend_from_slice(&[IAC, SE]);
    }
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream};
    use std::time::Instant;

    use super::*;
    use std::time::Duration;

    fn decode(input: &[u8]) -> (Vec<u8>, Vec<Event>) {
        let (mut data, mut events) = (Vec::new(), Vec::new());
        Decoder::default().decode(input, &mut data, &mut events);
        (data, events)
    }

    #[test]
    fn separa_dados_de_comandos() {
        let (data, events) = decode(b"ol\xff\xfb\x01\xc3\xa1\xff\xff!\r\0x\r\n\xff\xf1");
        assert_eq!(data, b"ol\xc3\xa1\xff!\rx\r\n");
        assert_eq!(events, [Event::Verb(WILL, ECHO)]);
        let (data, events) = decode(b"\xff\xfa\x18\x01\xff\xf0fim");
        assert_eq!(data, b"fim");
        assert_eq!(events, [Event::Sub(TTYPE, vec![TTYPE_SEND])]);
    }

    #[test]
    fn decodifica_em_pedacos() {
        let mut decoder = Decoder::default();
        let (mut data, mut events) = (Vec::new(), Vec::new());
        for byte in b"a\xff\xfd\x1fb" {
            decoder.decode(&[*byte], &mut data, &mut events);
        }
        assert_eq!(data, b"ab");
        assert_eq!(events, [Event::Verb(DO, NAWS)]);
    }

    #[test]
    fn enter_vira_cr_lf_e_iac_dobra() {
        let mut out = Vec::new();
        encode(b"ls\r\xff\r\n", &mut out);
        assert_eq!(out, b"ls\r\n\xff\xff\r\n");
    }

    #[test]
    fn negocia_eco_sga_naws_e_ttype() {
        let mut n = Negotiation::default();
        let mut out = Vec::new();
        n.start(&mut out);
        assert_eq!(
            out,
            [
                IAC, WILL, NAWS, IAC, WILL, TTYPE, IAC, DO, ECHO, IAC, DO, SGA
            ]
        );

        // Respostas aos pedidos do terminal: nada a mandar de volta.
        out.clear();
        n.resize(80, 24, &mut out);
        assert!(out.is_empty(), "o NAWS espera o DO do servidor");
        n.receive(Event::Verb(WILL, ECHO), &mut out);
        n.receive(Event::Verb(WILL, SGA), &mut out);
        n.receive(Event::Verb(DO, NAWS), &mut out);
        assert_eq!(out, [IAC, SB, NAWS, 0, 80, 0, 24, IAC, SE]);

        // O tamanho de novo a cada mudança, com o 255 dobrado.
        out.clear();
        n.resize(255, 50, &mut out);
        assert_eq!(out, [IAC, SB, NAWS, 0, 255, 255, 0, 50, IAC, SE]);

        out.clear();
        n.receive(Event::Verb(DO, TTYPE), &mut out);
        n.receive(Event::Sub(TTYPE, vec![TTYPE_SEND]), &mut out);
        assert_eq!(out, b"\xff\xfa\x18\x00XTERM-256COLOR\xff\xf0");

        // O que o terminal não sabe fazer, recusa.
        out.clear();
        n.receive(Event::Verb(DO, 32), &mut out);
        n.receive(Event::Verb(WILL, 39), &mut out);
        assert_eq!(out, [IAC, WONT, 32, IAC, DONT, 39]);
    }

    #[test]
    fn nao_entra_em_laco_com_um_eco() {
        // Um servidor que só repete devolve os pedidos do terminal como se fossem dele.
        let mut n = Negotiation::default();
        let mut out = Vec::new();
        n.start(&mut out);
        let mut echoed = out.clone();
        for _ in 0..4 {
            let (_, events) = decode(&echoed);
            let mut replies = Vec::new();
            for event in events {
                n.receive(event, &mut replies);
            }
            if replies.is_empty() {
                return;
            }
            echoed = replies;
        }
        panic!("a negociação não terminou: {echoed:?}");
    }

    fn wait_for(
        transport: &mut impl Transport,
        buf: &mut Vec<u8>,
        until: impl Fn(&[u8]) -> bool,
    ) -> Status {
        let start = Instant::now();
        loop {
            let status = transport.read(buf, usize::MAX);
            if until(buf) || status != Status::Open {
                return status;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "esperando: {buf:?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn conversa_com_um_servidor_de_verdade() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .write_all(b"\xff\xfd\x1f\xff\xfb\x01login: ")
                .unwrap();
            let mut received = Vec::new();
            let mut buf = [0; 256];
            while !received.ends_with(b"ana\r\n") {
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0, "o cliente fechou: {received:?}");
                received.extend_from_slice(&buf[..n]);
            }
            socket.write_all(b"ana\r\nbem-vinda\r\n").unwrap();
            received
        });

        let mut telnet = Telnet::connect("127.0.0.1", port);
        telnet.resize(100, 30);
        let mut buf = Vec::new();
        wait_for(&mut telnet, &mut buf, |b| b.ends_with(b"login: "));
        assert_eq!(buf, b"login: ");
        telnet.write(b"ana\r").unwrap();
        wait_for(&mut telnet, &mut buf, |b| b.ends_with(b"bem-vinda\r\n"));
        let received = server.join().unwrap();
        let naws: &[u8] = &[IAC, SB, NAWS, 0, 100, 0, 30, IAC, SE];
        assert!(
            received.windows(naws.len()).any(|w| w == naws),
            "o NAWS com 100 × 30: {received:?}"
        );
        assert!(received.starts_with(&[IAC, WILL, NAWS]));
        // O servidor fechou: a conexão termina sem erro.
        assert_eq!(
            wait_for(&mut telnet, &mut buf, |_| false),
            Status::Closed(None)
        );
    }

    #[test]
    fn conexao_recusada_aparece_como_motivo() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let mut telnet = Telnet::connect("127.0.0.1", port);
        let mut buf = Vec::new();
        assert_eq!(
            wait_for(&mut telnet, &mut buf, |_| false),
            Status::Closed(Some("conexão recusada".into()))
        );
    }

    #[test]
    fn reopen_e_duplicate_conectam_de_novo() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            // A primeira conexão cai logo; a segunda (reopen), a terceira (duplicate) e a quarta
            // (duplicate_in, que ignora a pasta) falam.
            let (mut primeira, _) = listener.accept().unwrap();
            primeira.write_all(b"um").unwrap();
            drop(primeira);
            let mut resto = Vec::new();
            for texto in [&b"dois"[..], b"tres", b"quatro"] {
                let (mut socket, _) = listener.accept().unwrap();
                socket.write_all(texto).unwrap();
                resto.push(socket);
            }
            resto
        });

        let mut telnet = Telnet::connect("127.0.0.1", port);
        let mut buf = Vec::new();
        wait_for(&mut telnet, &mut buf, |b| b == b"um");
        // O servidor fecha com dados do cliente ainda por ler (a negociação do Telnet), então o
        // sistema pode devolver um RST em vez do fim limpo: qualquer encerramento serve.
        assert!(matches!(
            wait_for(&mut telnet, &mut buf, |_| false),
            Status::Closed(_)
        ));

        assert!(telnet.reopen());
        buf.clear();
        wait_for(&mut telnet, &mut buf, |b| b == b"dois");

        let mut copia = telnet.duplicate().expect("o Telnet sabe duplicar");
        buf.clear();
        wait_for(&mut copia, &mut buf, |b| b == b"tres");
        // O Telnet não tem pasta: duplicar numa pasta é duplicar.
        let mut outra = telnet
            .duplicate_in(Some(std::path::Path::new("/tmp")))
            .expect("o Telnet sabe duplicar");
        assert_eq!(telnet.process_id(), None);
        buf.clear();
        wait_for(&mut outra, &mut buf, |b| b == b"quatro");
        drop(server.join().unwrap());
    }

    #[test]
    fn o_builder_passa_pelo_proxy_e_roda_o_gancho_antes() {
        let servidor = TcpListener::bind("127.0.0.1:0").unwrap();
        let porta_do_servidor = servidor.local_addr().unwrap().port();
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let porta_do_proxy = proxy.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut socket, _) = servidor.accept().unwrap();
            socket.write_all(b"bem-vinda").unwrap();
            let mut buf = [0u8; 64];
            while socket.read(&mut buf).is_ok_and(|n| n > 0) {}
        });
        // O proxy ignora o destino pedido (`10.0.0.5:23`) e liga ao servidor de teste.
        let pedido = thread::spawn(move || {
            let (mut cliente, _) = proxy.accept().unwrap();
            let mut saudacao = [0u8; 3];
            cliente.read_exact(&mut saudacao).unwrap();
            cliente.write_all(&[5, 0]).unwrap();
            let mut inicio = [0u8; 5];
            cliente.read_exact(&mut inicio).unwrap();
            let mut resto = vec![0u8; usize::from(inicio[4]) + 2];
            cliente.read_exact(&mut resto).unwrap();
            cliente.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
            let servidor = TcpStream::connect(("127.0.0.1", porta_do_servidor)).unwrap();
            let (mut ida, mut volta) =
                (cliente.try_clone().unwrap(), servidor.try_clone().unwrap());
            thread::spawn(move || {
                let _ = io::copy(&mut ida, &mut volta);
            });
            let (mut origem, mut destino) = (servidor, cliente);
            thread::spawn(move || {
                let _ = io::copy(&mut origem, &mut destino);
            });
            String::from_utf8_lossy(&resto[..resto.len() - 2]).into_owned()
        });
        let mut telnet = Telnet::builder("10.0.0.5", 23)
            .proxy(Proxy::Socks5 {
                host: "127.0.0.1".into(),
                port: porta_do_proxy,
                user: None,
                password: None,
            })
            .before_connect(|aviso| {
                aviso("Subindo a rede…");
                Ok(())
            })
            .connect();
        let mut buf = Vec::new();
        wait_for(&mut telnet, &mut buf, |b| b.ends_with(b"bem-vinda"));
        assert!(buf.starts_with("Subindo a rede…\r\n".as_bytes()), "{buf:?}");
        assert_eq!(pedido.join().unwrap(), "10.0.0.5");
    }

    #[test]
    fn o_gancho_com_erro_fecha_a_sessao_sem_conectar() {
        let servidor = TcpListener::bind("127.0.0.1:0").unwrap();
        servidor.set_nonblocking(true).unwrap();
        let porta = servidor.local_addr().unwrap().port();
        let mut telnet = Telnet::builder("127.0.0.1", porta)
            .before_connect(|_| Err("a rede não subiu".to_owned()))
            .connect();
        let mut buf = Vec::new();
        assert_eq!(
            wait_for(&mut telnet, &mut buf, |_| false),
            Status::Closed(Some("a rede não subiu".into()))
        );
        assert_eq!(
            servidor.accept().map(|_| ()).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn conectando_ate_o_tcp_abrir_e_de_novo_no_reopen() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket.write_all(b"oi").unwrap();
                let mut sobra = Vec::new();
                let _ = socket.read_to_end(&mut sobra);
            }
        });
        // O gancho segura a conexão até o teste liberar: antes disso, ela está abrindo.
        let (liberar, espera) = mpsc::channel::<()>();
        let espera = Mutex::new(espera);
        let mut telnet = Telnet::builder("127.0.0.1", port)
            .before_connect(move |_| {
                let _ = espera.lock().unwrap().recv();
                Ok(())
            })
            .connect();
        assert!(telnet.connecting());
        liberar.send(()).unwrap();
        let mut buf = Vec::new();
        wait_for(&mut telnet, &mut buf, |b| b.ends_with(b"oi"));
        assert!(!telnet.connecting());

        // Cai e reabre: o gancho roda de novo e a conexão volta a abrir.
        let _ = telnet.commands.send(Command::Close);
        wait_for(&mut telnet, &mut buf, |_| false);
        assert!(telnet.reopen());
        assert!(telnet.connecting());
        liberar.send(()).unwrap();
        let mut depois = Vec::new();
        wait_for(&mut telnet, &mut depois, |b| b.ends_with(b"oi"));
        assert!(!telnet.connecting());
        drop(telnet);
        server.join().unwrap();
    }

    #[test]
    fn conexao_recusada_deixa_de_conectar() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let mut telnet = Telnet::connect("127.0.0.1", port);
        let mut buf = Vec::new();
        wait_for(&mut telnet, &mut buf, |_| false);
        let limite = Instant::now() + Duration::from_secs(5);
        while telnet.connecting() && Instant::now() < limite {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!telnet.connecting());
    }
}
