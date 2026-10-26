//! O proxy por onde uma conexão chega ao servidor: SOCKS5 (RFC 1928, com usuário e senha pela
//! RFC 1929) e HTTP CONNECT (com `Proxy-Authorization: Basic`). Escrito à mão e síncrono: roda
//! na thread da conexão, antes de o `tokio` assumir o `TcpStream`.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::{base64, connect_error_message, connect_tcp};

/// Quanto o proxy tem para responder em cada passo da conversa.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// O maior cabeçalho de resposta do HTTP CONNECT que se aceita ler.
const MAX_HEADER: usize = 8 * 1024;

/// O proxy por onde uma conexão chega ao servidor (no SSH, ao primeiro da cadeia).
///
/// ```no_run
/// use tomni_ui_net::{CONNECT_TIMEOUT, Proxy, connect};
///
/// let proxy = Proxy::Socks5 {
///     host: "proxy.exemplo".into(),
///     port: 1080,
///     user: None,
///     password: None,
/// };
/// let tcp = connect("interno.exemplo", 22, Some(&proxy), CONNECT_TIMEOUT);
/// ```
#[derive(Clone, PartialEq, Eq)]
pub enum Proxy {
    /// SOCKS5. Com `user` e `password`, usa a autenticação por usuário e senha da RFC 1929.
    Socks5 {
        /// O servidor do proxy.
        host: String,
        /// A porta (a padrão do SOCKS é 1080).
        port: u16,
        /// O usuário, se o proxy pede.
        user: Option<String>,
        /// A senha.
        password: Option<String>,
    },
    /// HTTP CONNECT. Com `user` e `password`, manda o `Proxy-Authorization: Basic`.
    Http {
        /// O servidor do proxy.
        host: String,
        /// A porta (8080 é comum).
        port: u16,
        /// O usuário, se o proxy pede.
        user: Option<String>,
        /// A senha.
        password: Option<String>,
    },
}

impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, host, port, user, password) = self.parts();
        f.debug_struct(kind)
            .field("host", &host)
            .field("port", &port)
            .field("user", &user)
            .field("password", &password.map(|_| "…"))
            .finish()
    }
}

impl Proxy {
    fn parts(&self) -> (&'static str, &str, u16, Option<&str>, Option<&str>) {
        match self {
            Self::Socks5 {
                host,
                port,
                user,
                password,
            } => ("Socks5", host, *port, user.as_deref(), password.as_deref()),
            Self::Http {
                host,
                port,
                user,
                password,
            } => ("Http", host, *port, user.as_deref(), password.as_deref()),
        }
    }

    /// Abre a conexão com o proxy e pede o túnel até `host:port`. Devolve o socket já ligado ao
    /// servidor de destino, ou a mensagem em português do que deu errado. `timeout` vale para
    /// chegar ao proxy.
    pub fn open(&self, host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
        let (_, proxy_host, proxy_port, user, password) = self.parts();
        let mut stream = connect_tcp(proxy_host, proxy_port, timeout)
            .map_err(|error| format!("proxy {proxy_host}: {}", connect_error_message(&error)))?;
        let _ = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
        let _ = stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT));
        let result = match self {
            Self::Socks5 { .. } => socks5(&mut stream, host, port, user.zip(password)),
            Self::Http { .. } => http_connect(&mut stream, host, port, user.zip(password)),
        };
        result.map_err(|error| match error {
            Failure::Io(error) => format!("proxy {proxy_host}: {}", connect_error_message(&error)),
            Failure::Proxy(message) => message,
        })?;
        let _ = stream.set_read_timeout(None);
        let _ = stream.set_write_timeout(None);
        Ok(stream)
    }
}

/// Por que a conversa com o proxy falhou.
enum Failure {
    /// Um erro do socket (ou o proxy fechou no meio).
    Io(io::Error),
    /// O proxy respondeu, mas recusou ou falou fora do protocolo.
    Proxy(String),
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn refused(code: impl fmt::Display) -> Failure {
    Failure::Proxy(format!("o proxy recusou a conexão (código {code})"))
}

/// O código de resposta do SOCKS5 (RFC 1928, seção 6) com o motivo, quando a RFC dá um.
fn socks5_code(code: u8) -> String {
    let reason = match code {
        1 => "falha geral do servidor SOCKS",
        2 => "conexão não permitida pelo proxy",
        3 => "rede inalcançável",
        4 => "host inalcançável",
        5 => "conexão recusada pelo destino",
        6 => "tempo esgotado",
        7 => "comando não suportado",
        8 => "tipo de endereço não suportado",
        _ => return code.to_string(),
    };
    format!("{code}: {reason}")
}

fn bad_credentials() -> Failure {
    Failure::Proxy("o proxy recusou o usuário ou a senha".to_owned())
}

fn wants_authentication() -> Failure {
    Failure::Proxy("o proxy pediu autenticação".to_owned())
}

fn malformed() -> Failure {
    Failure::Proxy("o proxy respondeu fora do protocolo".to_owned())
}

/// SOCKS5: a negociação do método, a autenticação da RFC 1929 e o pedido de CONNECT.
fn socks5(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    credentials: Option<(&str, &str)>,
) -> Result<(), Failure> {
    if credentials.is_some() {
        stream.write_all(&[5, 2, 0, 2])?;
    } else {
        stream.write_all(&[5, 1, 0])?;
    }
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply)?;
    if reply[0] != 5 {
        return Err(malformed());
    }
    match (reply[1], credentials) {
        (0, _) => {}
        (2, Some((user, password))) => {
            let (user, password) = (user.as_bytes(), password.as_bytes());
            let (Ok(user_len), Ok(password_len)) =
                (u8::try_from(user.len()), u8::try_from(password.len()))
            else {
                return Err(Failure::Proxy(
                    "o usuário e a senha do proxy cabem em 255 bytes cada".to_owned(),
                ));
            };
            let mut auth = vec![1, user_len];
            auth.extend_from_slice(user);
            auth.push(password_len);
            auth.extend_from_slice(password);
            stream.write_all(&auth)?;
            let mut status = [0u8; 2];
            stream.read_exact(&mut status)?;
            if status[1] != 0 {
                return Err(bad_credentials());
            }
        }
        (0xFF, _) | (2, None) => return Err(wants_authentication()),
        _ => return Err(malformed()),
    }

    let Ok(host_len) = u8::try_from(host.len()) else {
        return Err(Failure::Proxy(
            "o nome do servidor é longo demais".to_owned(),
        ));
    };
    let mut request = vec![5, 1, 0, 3, host_len];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request)?;

    let mut head = [0u8; 4];
    stream.read_exact(&mut head)?;
    if head[0] != 5 {
        return Err(malformed());
    }
    if head[1] != 0 {
        return Err(refused(socks5_code(head[1])));
    }
    // O endereço que o proxy ligou: não interessa, mas precisa sair do socket.
    let skip = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len)?;
            usize::from(len[0])
        }
        _ => return Err(malformed()),
    };
    let mut rest = vec![0u8; skip + 2];
    stream.read_exact(&mut rest)?;
    Ok(())
}

/// HTTP CONNECT: o pedido, e a resposta lida byte a byte até a linha em branco (para não
/// engolir o que o servidor de destino mandar logo depois).
fn http_connect(
    stream: &mut TcpStream,
    host: &str,
    port: u16,
    credentials: Option<(&str, &str)>,
) -> Result<(), Failure> {
    let target = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some((user, password)) = credentials {
        let token = base64(format!("{user}:{password}").as_bytes());
        request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes())?;

    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HEADER {
            return Err(malformed());
        }
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head);
    let status = text.lines().next().unwrap_or_default();
    let mut words = status.split_whitespace();
    let (Some(version), Some(code)) = (words.next(), words.next()) else {
        return Err(malformed());
    };
    if !version.starts_with("HTTP/1.") {
        return Err(malformed());
    }
    match code {
        "200" => Ok(()),
        "407" if credentials.is_some() => Err(bad_credentials()),
        "407" => Err(wants_authentication()),
        other => Err(refused(other)),
    }
}
