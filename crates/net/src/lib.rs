//! Rede do Tomni UI (M30): o [`Proxy`] (SOCKS5 e HTTP CONNECT) e a abertura da conexão TCP
//! (`connect`), com as mensagens de erro em português. É a base do SSH, do Telnet e, depois, do
//! FTP e do S3; não depende de nenhuma outra parte da lib. O [`HttpProxyServer`] é o outro
//! lado: um proxy HTTP local que leva o navegador por um caminho que o app escolhe.

#[cfg(feature = "http")]
pub mod http;
mod proxy;
mod proxy_server;

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub use proxy::Proxy;
pub use proxy_server::{Dial, HttpProxyServer};

/// Quanto se espera, por padrão, para o TCP chegar ao servidor.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Abre a conexão TCP até `host:port`, pelo `proxy` se houver. Tenta cada endereço de `host`
/// por até `timeout`. O erro vem com a mensagem em português ([`connect_error_message`]); no
/// proxy, com o que ele respondeu.
///
/// ```no_run
/// use std::time::Duration;
/// use tomni_ui_net::{connect, CONNECT_TIMEOUT};
///
/// let tcp = connect("servidor.exemplo", 22, None, CONNECT_TIMEOUT);
/// ```
pub fn connect(
    host: &str,
    port: u16,
    proxy: Option<&Proxy>,
    timeout: Duration,
) -> io::Result<TcpStream> {
    match proxy {
        Some(proxy) => proxy.open(host, port, timeout).map_err(io::Error::other),
        None => connect_tcp(host, port, timeout),
    }
}

/// Abre a conexão TCP direta, tentando cada endereço de `host` por até `timeout`.
pub fn connect_tcp(host: &str, port: u16, timeout: Duration) -> io::Result<TcpStream> {
    let addrs = (host, port).to_socket_addrs().map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("servidor não encontrado: {host}"),
        )
    })?;
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("servidor não encontrado: {host}"),
        )
    }))
}

/// Mensagem em português para um erro de conexão.
pub fn error_message(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::ConnectionRefused => "conexão recusada".to_owned(),
        io::ErrorKind::ConnectionReset => "conexão interrompida pelo outro lado".to_owned(),
        io::ErrorKind::ConnectionAborted => "conexão interrompida".to_owned(),
        io::ErrorKind::TimedOut => "tempo de conexão esgotado".to_owned(),
        io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable => {
            "servidor inalcançável".to_owned()
        }
        io::ErrorKind::PermissionDenied => "permissão negada".to_owned(),
        io::ErrorKind::NotFound => "não encontrado".to_owned(),
        io::ErrorKind::BrokenPipe => "a conexão foi encerrada".to_owned(),
        _ => error.to_string(),
    }
}

/// A mensagem de um erro do [`connect_tcp`]: o "servidor não encontrado" já vem pronto.
pub fn connect_error_message(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => error.to_string(),
        _ => error_message(error),
    }
}

/// Base64 padrão (RFC 4648), só para o `Proxy-Authorization`.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_confere_com_os_vetores_da_rfc_4648() {
        for (entrada, esperado) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
            ("ana:senha", "YW5hOnNlbmhh"),
        ] {
            assert_eq!(base64(entrada.as_bytes()), esperado, "{entrada:?}");
        }
    }

    #[test]
    fn servidor_que_nao_existe_vira_nao_encontrado() {
        let erro = connect("nao-existe.invalid", 22, None, Duration::from_secs(2)).unwrap_err();
        assert_eq!(erro.kind(), io::ErrorKind::NotFound);
        assert_eq!(
            connect_error_message(&erro),
            "servidor não encontrado: nao-existe.invalid"
        );
    }

    #[test]
    fn porta_fechada_vira_conexao_recusada() {
        let porta = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let erro = connect("127.0.0.1", porta, None, CONNECT_TIMEOUT).unwrap_err();
        assert_eq!(connect_error_message(&erro), "conexão recusada");
    }
}
