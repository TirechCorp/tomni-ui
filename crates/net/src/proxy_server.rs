//! O servidor de proxy HTTP local: escuta em `127.0.0.1` e leva cada pedido do navegador por
//! uma função de discagem (`dial`) que a aplicação dá: direto, por um [`crate::Proxy`], pela
//! ponte SOCKS de uma VPN embutida. Entende o `CONNECT` (o HTTPS e os WebSockets) e o pedido em
//! forma absoluta (`GET http://servidor/caminho`, o HTTP puro), que segue para o servidor em
//! forma de origem e com `Connection: close`. Síncrono, uma thread por sentido de cada conexão.
//!
//! Não pede usuário e senha: o Chromium e o Firefox não aceitam a credencial do proxy na linha
//! de comando e perguntariam numa janela. Ouve só no `127.0.0.1`.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::connect_error_message;

/// Quanto o navegador tem para mandar o cabeçalho do pedido.
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// O maior cabeçalho de pedido que se aceita.
const MAX_HEAD: usize = 64 * 1024;
/// Os cabeçalhos que são do proxy e não seguem para o servidor (e o `Connection`, que vira
/// `close`).
const HOP_HEADERS: [&str; 5] = [
    "proxy-connection",
    "proxy-authorization",
    "connection",
    "keep-alive",
    "te",
];

/// A função que abre a conexão TCP até `host:port`: é ela que decide o caminho (direto, proxy,
/// VPN). O nome chega sem resolver, para ser resolvido do lado de lá.
pub type Dial = dyn Fn(&str, u16) -> io::Result<TcpStream> + Send + Sync;

/// As conexões abertas, para o [`HttpProxyServer`] fechar todas ao parar.
type Open = Mutex<HashMap<u64, [TcpStream; 2]>>;

/// Um proxy HTTP local que leva cada conexão pela função `dial`. Para quando é solto (ou com
/// [`HttpProxyServer::stop`]) e derruba os túneis abertos.
///
/// ```no_run
/// use tomni_ui_net::{CONNECT_TIMEOUT, HttpProxyServer, connect_tcp};
///
/// let proxy = HttpProxyServer::start(|host, port| connect_tcp(host, port, CONNECT_TIMEOUT))?;
/// println!("--proxy-server={}", proxy.url());
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct HttpProxyServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    open: Arc<Open>,
    accept: Option<JoinHandle<()>>,
}

impl HttpProxyServer {
    /// Abre o proxy numa porta livre do `127.0.0.1`.
    pub fn start(
        dial: impl Fn(&str, u16) -> io::Result<TcpStream> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        Self::bind("127.0.0.1:0", dial)
    }

    /// Abre o proxy em `addr` (uma porta fixa, por exemplo).
    pub fn bind(
        addr: impl ToSocketAddrs,
        dial: impl Fn(&str, u16) -> io::Result<TcpStream> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let open: Arc<Open> = Arc::default();
        let dial: Arc<Dial> = Arc::new(dial);
        let accept = {
            let (stop, open) = (Arc::clone(&stop), Arc::clone(&open));
            thread::Builder::new()
                .name("tomni-http-proxy".to_owned())
                .spawn(move || accept_loop(&listener, &stop, &open, &dial))?
        };
        Ok(Self {
            addr,
            stop,
            open,
            accept: Some(accept),
        })
    }

    /// O endereço em que o proxy ouve.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// O endereço no formato do `--proxy-server` do Chromium: `http://127.0.0.1:porta`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Quantas conexões passam pelo proxy agora.
    pub fn connections(&self) -> usize {
        lock(&self.open).len()
    }

    /// Para de aceitar e fecha as conexões abertas. É o mesmo que soltar.
    pub fn stop(self) {}
}

impl Drop for HttpProxyServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Acorda o `accept`, que está parado esperando alguém.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        for [a, b] in lock(&self.open).drain().map(|(_, s)| s) {
            let _ = a.shutdown(Shutdown::Both);
            let _ = b.shutdown(Shutdown::Both);
        }
    }
}

fn lock(open: &Open) -> std::sync::MutexGuard<'_, HashMap<u64, [TcpStream; 2]>> {
    open.lock().unwrap_or_else(PoisonError::into_inner)
}

fn accept_loop(listener: &TcpListener, stop: &Arc<AtomicBool>, open: &Arc<Open>, dial: &Arc<Dial>) {
    let ids = Arc::new(AtomicU64::new(0));
    for client in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(client) = client else { continue };
        let (stop, open, dial, ids) = (
            Arc::clone(stop),
            Arc::clone(open),
            Arc::clone(dial),
            Arc::clone(&ids),
        );
        let _ = thread::Builder::new()
            .name("tomni-http-proxy-conn".to_owned())
            .spawn(move || {
                let id = ids.fetch_add(1, Ordering::Relaxed);
                serve(client, &dial, &open, &stop, id);
            });
    }
}

/// Um pedido lido: a linha de pedido, os cabeçalhos e o que já veio depois deles.
struct Request {
    method: String,
    target: String,
    version: String,
    headers: Vec<(String, String)>,
    rest: Vec<u8>,
}

/// Por que o pedido não segue: o código e o texto da resposta.
struct Reject(u16, &'static str, String);

fn serve(mut client: TcpStream, dial: &Arc<Dial>, open: &Open, stop: &AtomicBool, id: u64) {
    let _ = client.set_read_timeout(Some(HEAD_TIMEOUT));
    let result = read_request(&mut client).and_then(|request| route(request, dial.as_ref()));
    let (mut server, head) = match result {
        Ok(ok) => ok,
        Err(Reject(code, reason, message)) => {
            let _ = respond(&mut client, code, reason, &message);
            return;
        }
    };
    let _ = client.set_read_timeout(None);
    if client.write_all(&head.to_client).is_err() || server.write_all(&head.to_server).is_err() {
        return;
    }
    let (Ok(client_copy), Ok(server_copy)) = (client.try_clone(), server.try_clone()) else {
        return;
    };
    lock(open).insert(id, [client_copy, server_copy]);
    // O `stop` pode ter chegado entre o `accept` e o registro: a conexão não fica para trás.
    if stop.load(Ordering::SeqCst) {
        lock(open).remove(&id);
        return;
    }
    pipe(client, server);
    lock(open).remove(&id);
}

/// O que se manda a cada lado antes de ligar os dois.
struct Head {
    to_client: Vec<u8>,
    to_server: Vec<u8>,
}

/// Decide o destino do pedido, disca e monta o que vai a cada lado.
fn route(request: Request, dial: &Dial) -> Result<(TcpStream, Head), Reject> {
    if !request.version.starts_with("HTTP/1.") {
        return Err(bad_request("versão do HTTP não suportada"));
    }
    if request.method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = authority(&request.target, 443)
            .ok_or_else(|| bad_request("destino inválido no CONNECT"))?;
        let server = open_server(dial, &host, port)?;
        let head = Head {
            to_client: b"HTTP/1.1 200 Connection Established\r\n\r\n".to_vec(),
            to_server: request.rest,
        };
        return Ok((server, head));
    }
    let Some(url) = strip_scheme(&request.target, "http://") else {
        return Err(Reject(
            501,
            "Not Implemented",
            "este proxy só leva o CONNECT e o HTTP".to_owned(),
        ));
    };
    let (authority_part, path) = match url.find(['/', '?']) {
        Some(at) if url.as_bytes()[at] == b'?' => (&url[..at], format!("/{}", &url[at..])),
        Some(at) => (&url[..at], url[at..].to_owned()),
        None => (url, "/".to_owned()),
    };
    let (host, port) =
        authority(authority_part, 80).ok_or_else(|| bad_request("endereço inválido"))?;
    let mut to_server = format!("{} {path} {}\r\n", request.method, request.version);
    for (name, value) in &request.headers {
        if !HOP_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            to_server.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    to_server.push_str("Connection: close\r\n\r\n");
    let mut to_server = to_server.into_bytes();
    to_server.extend_from_slice(&request.rest);
    let server = open_server(dial, &host, port)?;
    Ok((
        server,
        Head {
            to_client: Vec::new(),
            to_server,
        },
    ))
}

fn bad_request(message: &str) -> Reject {
    Reject(400, "Bad Request", message.to_owned())
}

fn open_server(dial: &Dial, host: &str, port: u16) -> Result<TcpStream, Reject> {
    dial(host, port).map_err(|error| {
        Reject(
            502,
            "Bad Gateway",
            format!("{host}:{port}: {}", connect_error_message(&error)),
        )
    })
}

fn strip_scheme<'a>(target: &'a str, scheme: &str) -> Option<&'a str> {
    let head = target.get(..scheme.len())?;
    head.eq_ignore_ascii_case(scheme)
        .then(|| &target[scheme.len()..])
}

/// `host`, `host:porta`, `[::1]:porta` (e um `usuário@` na frente, que se descarta).
fn authority(text: &str, default_port: u16) -> Option<(String, u16)> {
    let text = text.rsplit_once('@').map_or(text, |(_, host)| host);
    let (host, port) = if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(port) => port.parse().ok()?,
            None if after.is_empty() => default_port,
            None => return None,
        };
        (host, port)
    } else {
        match text.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok()?),
            None => (text, default_port),
        }
    };
    (!host.is_empty() && !host.contains(char::is_whitespace)).then(|| (host.to_owned(), port))
}

/// Lê até a linha em branco; o que vier junto fica em `rest`.
fn read_request(client: &mut TcpStream) -> Result<Request, Reject> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if buffer.len() >= MAX_HEAD {
            return Err(Reject(
                431,
                "Request Header Fields Too Large",
                "cabeçalho grande demais".to_owned(),
            ));
        }
        match client.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(bad_request("o pedido veio incompleto")),
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    };
    let rest = buffer.split_off(end + 4);
    let text = String::from_utf8(buffer).map_err(|_| bad_request("o cabeçalho não é texto"))?;
    let mut lines = text.split("\r\n");
    let mut words = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (words.next(), words.next(), words.next(), words.next())
    else {
        return Err(bad_request("linha de pedido inválida"));
    };
    let headers = lines
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
                .ok_or_else(|| bad_request("cabeçalho inválido"))
        })
        .collect::<Result<_, _>>()?;
    Ok(Request {
        method: method.to_owned(),
        target: target.to_owned(),
        version: version.to_owned(),
        headers,
        rest,
    })
}

fn respond(client: &mut TcpStream, code: u16, reason: &str, message: &str) -> io::Result<()> {
    let body = format!("{message}\n");
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    client.write_all(head.as_bytes())?;
    client.write_all(body.as_bytes())
}

/// Liga os dois lados até os dois fecharem: cada sentido numa thread, e o fim de um sentido
/// fecha a escrita do outro lado (o meio-fechamento do TCP).
fn pipe(client: TcpStream, server: TcpStream) {
    let (Ok(mut client_read), Ok(mut server_write)) = (client.try_clone(), server.try_clone())
    else {
        return;
    };
    let upload = thread::Builder::new()
        .name("tomni-http-proxy-up".to_owned())
        .spawn(move || {
            let _ = io::copy(&mut client_read, &mut server_write);
            let _ = server_write.shutdown(Shutdown::Write);
        });
    let (mut server_read, mut client_write) = (server, client);
    let _ = io::copy(&mut server_read, &mut client_write);
    let _ = client_write.shutdown(Shutdown::Write);
    if let Ok(upload) = upload {
        let _ = upload.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect_tcp;
    use std::io::BufRead;
    use std::sync::mpsc;

    const TIMEOUT: Duration = Duration::from_secs(5);

    /// Um servidor de eco numa porta livre.
    fn echo() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                thread::spawn(move || {
                    let mut write = stream.try_clone().unwrap();
                    let mut read = stream;
                    let _ = io::copy(&mut read, &mut write);
                });
            }
        });
        addr
    }

    /// Os destinos que a discagem recebeu.
    type Dialed = mpsc::Receiver<(String, u16)>;

    /// A discagem dos testes: todo nome vai para `addr`, e o pedido fica anotado.
    fn dial_to(addr: SocketAddr) -> (impl Fn(&str, u16) -> io::Result<TcpStream>, Dialed) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        let dial = move |host: &str, port: u16| {
            let _ = lock_sender(&sender).send((host.to_owned(), port));
            connect_tcp("127.0.0.1", addr.port(), TIMEOUT)
        };
        (dial, receiver)
    }

    fn lock_sender<T>(sender: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        sender.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn to_proxy(proxy: &HttpProxyServer) -> TcpStream {
        let stream = TcpStream::connect(proxy.addr()).unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        stream
    }

    /// Lê a resposta do proxy até a linha em branco.
    fn read_head(stream: &mut io::BufReader<TcpStream>) -> String {
        let mut head = String::new();
        loop {
            let mut line = String::new();
            stream.read_line(&mut line).unwrap();
            head.push_str(&line);
            if line == "\r\n" || line.is_empty() {
                return head;
            }
        }
    }

    #[test]
    fn connect_leva_o_nome_sem_resolver_a_discagem_e_liga_os_dois_lados() {
        let (dial, pedidos) = dial_to(echo());
        let proxy = HttpProxyServer::start(dial).unwrap();
        assert!(proxy.url().starts_with("http://127.0.0.1:"));
        let mut stream = to_proxy(&proxy);
        stream
            .write_all(
                b"CONNECT intranet.cliente:8443 HTTP/1.1\r\nHost: intranet.cliente:8443\r\n\r\nola",
            )
            .unwrap();
        let mut reader = io::BufReader::new(stream.try_clone().unwrap());
        assert!(read_head(&mut reader).starts_with("HTTP/1.1 200"));
        let mut eco = [0u8; 3];
        reader.read_exact(&mut eco).unwrap();
        assert_eq!(&eco, b"ola", "o que veio junto do cabeçalho seguiu");
        stream.write_all(b" mundo").unwrap();
        let mut eco = [0u8; 6];
        reader.read_exact(&mut eco).unwrap();
        assert_eq!(&eco, b" mundo");
        assert_eq!(
            pedidos.recv_timeout(TIMEOUT).unwrap(),
            ("intranet.cliente".to_owned(), 8443)
        );
        assert_eq!(proxy.connections(), 1);
    }

    #[test]
    fn pedido_absoluto_segue_em_forma_de_origem_sem_os_cabecalhos_do_proxy() {
        let servidor = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = servidor.local_addr().unwrap();
        let recebido = thread::spawn(move || {
            let (mut stream, _) = servidor.accept().unwrap();
            stream.set_read_timeout(Some(TIMEOUT)).unwrap();
            let mut pedido = Vec::new();
            let mut chunk = [0u8; 1024];
            while !pedido.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut chunk).unwrap();
                pedido.extend_from_slice(&chunk[..n]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
            String::from_utf8(pedido).unwrap()
        });
        let (dial, pedidos) = dial_to(addr);
        let proxy = HttpProxyServer::start(dial).unwrap();
        let mut stream = to_proxy(&proxy);
        stream
            .write_all(
                b"GET http://wiki.cliente/pagina?x=1 HTTP/1.1\r\nHost: wiki.cliente\r\n\
                  Proxy-Connection: keep-alive\r\nProxy-Authorization: Basic eA==\r\n\
                  Accept: text/html\r\n\r\n",
            )
            .unwrap();
        let mut resposta = String::new();
        stream.read_to_string(&mut resposta).unwrap();
        assert!(resposta.ends_with("\r\n\r\nok"), "{resposta}");
        let pedido = recebido.join().unwrap();
        assert!(
            pedido.starts_with("GET /pagina?x=1 HTTP/1.1\r\n"),
            "{pedido}"
        );
        assert!(pedido.contains("Host: wiki.cliente\r\n"));
        assert!(pedido.contains("Accept: text/html\r\n"));
        assert!(pedido.contains("Connection: close\r\n"));
        assert!(!pedido.to_ascii_lowercase().contains("proxy-"), "{pedido}");
        assert_eq!(
            pedidos.recv_timeout(TIMEOUT).unwrap(),
            ("wiki.cliente".to_owned(), 80)
        );
    }

    #[test]
    fn discagem_que_falha_vira_502_com_a_mensagem() {
        let proxy =
            HttpProxyServer::start(|_, _| Err(io::Error::from(io::ErrorKind::ConnectionRefused)))
                .unwrap();
        let mut stream = to_proxy(&proxy);
        stream
            .write_all(b"CONNECT banco.cliente:5432 HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut resposta = String::new();
        stream.read_to_string(&mut resposta).unwrap();
        assert!(
            resposta.starts_with("HTTP/1.1 502 Bad Gateway\r\n"),
            "{resposta}"
        );
        assert!(
            resposta.ends_with("banco.cliente:5432: conexão recusada\n"),
            "{resposta}"
        );
    }

    #[test]
    fn pedido_estranho_vira_400_ou_501() {
        let proxy = HttpProxyServer::start(|_, _| unreachable!("não disca")).unwrap();
        for (pedido, codigo) in [
            ("lixo\r\n\r\n", "400"),
            ("GET /local HTTP/1.1\r\n\r\n", "501"),
            ("GET ftp://arquivos/x HTTP/1.1\r\n\r\n", "501"),
            ("CONNECT [::1 HTTP/1.1\r\n\r\n", "400"),
            ("GET http://x/ HTTP/2\r\n\r\n", "400"),
        ] {
            let mut stream = to_proxy(&proxy);
            stream.write_all(pedido.as_bytes()).unwrap();
            let mut resposta = String::new();
            stream.read_to_string(&mut resposta).unwrap();
            assert!(
                resposta.starts_with(&format!("HTTP/1.1 {codigo} ")),
                "{pedido:?}: {resposta}"
            );
        }
    }

    #[test]
    fn parar_fecha_os_tuneis_e_a_porta() {
        let (dial, _pedidos) = dial_to(echo());
        let proxy = HttpProxyServer::start(dial).unwrap();
        let addr = proxy.addr();
        let mut stream = to_proxy(&proxy);
        stream.write_all(b"CONNECT a:1 HTTP/1.1\r\n\r\n").unwrap();
        let mut reader = io::BufReader::new(stream.try_clone().unwrap());
        assert!(read_head(&mut reader).starts_with("HTTP/1.1 200"));
        proxy.stop();
        let mut resto = Vec::new();
        assert_eq!(
            reader.read_to_end(&mut resto).unwrap_or(0),
            0,
            "o túnel fechou"
        );
        assert!(
            TcpStream::connect_timeout(&addr, TIMEOUT).is_err(),
            "a porta fechou"
        );
    }

    #[test]
    fn autoridade_com_porta_ipv6_e_usuario() {
        assert_eq!(authority("a.b", 80), Some(("a.b".into(), 80)));
        assert_eq!(authority("a.b:8080", 80), Some(("a.b".into(), 8080)));
        assert_eq!(authority("[::1]:22", 80), Some(("::1".into(), 22)));
        assert_eq!(authority("[fe80::1]", 443), Some(("fe80::1".into(), 443)));
        assert_eq!(authority("ana@a.b:81", 80), Some(("a.b".into(), 81)));
        assert_eq!(authority("a.b:porta", 80), None);
        assert_eq!(authority("", 80), None);
        assert_eq!(authority("[::1", 80), None);
    }
}
