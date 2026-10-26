//! O `Proxy` contra servidores de proxy locais (SOCKS5 e HTTP CONNECT) e um servidor de eco.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use tomni_ui_net::{CONNECT_TIMEOUT, Proxy, base64, connect};

/// Um servidor que devolve o que recebe, uma conexão só.
fn eco() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut cliente, _) = listener.accept().unwrap();
        let mut buf = [0u8; 64];
        while let Ok(n) = cliente.read(&mut buf) {
            if n == 0 || cliente.write_all(&buf[..n]).is_err() {
                break;
            }
        }
    });
    porta
}

/// Liga o cliente ao destino nas duas direções.
fn ponte(cliente: TcpStream, destino: TcpStream) {
    let (mut c1, mut d1) = (cliente.try_clone().unwrap(), destino.try_clone().unwrap());
    thread::spawn(move || {
        let _ = std::io::copy(&mut c1, &mut d1);
    });
    let (mut c2, mut d2) = (cliente, destino);
    thread::spawn(move || {
        let _ = std::io::copy(&mut d2, &mut c2);
    });
}

/// Um proxy SOCKS5 de teste; com `credenciais`, exige usuário e senha (RFC 1929).
fn socks5(credenciais: Option<(&'static str, &'static str)>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut cliente, _) = listener.accept().unwrap();
        let mut oferta = [0u8; 2];
        cliente.read_exact(&mut oferta).unwrap();
        let mut metodos = vec![0u8; usize::from(oferta[1])];
        cliente.read_exact(&mut metodos).unwrap();
        if let Some((usuario, senha)) = credenciais {
            cliente.write_all(&[5, 2]).unwrap();
            let mut cab = [0u8; 2];
            cliente.read_exact(&mut cab).unwrap();
            let mut u = vec![0u8; usize::from(cab[1])];
            cliente.read_exact(&mut u).unwrap();
            let mut tam = [0u8; 1];
            cliente.read_exact(&mut tam).unwrap();
            let mut s = vec![0u8; usize::from(tam[0])];
            cliente.read_exact(&mut s).unwrap();
            if u != usuario.as_bytes() || s != senha.as_bytes() {
                cliente.write_all(&[1, 1]).unwrap();
                return;
            }
            cliente.write_all(&[1, 0]).unwrap();
        } else {
            cliente.write_all(&[5, 0]).unwrap();
        }
        let mut pedido = [0u8; 5];
        cliente.read_exact(&mut pedido).unwrap();
        let mut host = vec![0u8; usize::from(pedido[4])];
        cliente.read_exact(&mut host).unwrap();
        let mut porta = [0u8; 2];
        cliente.read_exact(&mut porta).unwrap();
        let destino = TcpStream::connect((
            String::from_utf8(host).unwrap().as_str(),
            u16::from_be_bytes(porta),
        ))
        .unwrap();
        cliente.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        ponte(cliente, destino);
    });
    porta
}

/// Um proxy HTTP CONNECT de teste; com `credenciais`, exige o `Proxy-Authorization: Basic`.
fn http(credenciais: Option<(&'static str, &'static str)>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut cliente, _) = listener.accept().unwrap();
        let mut cab = Vec::new();
        let mut byte = [0u8; 1];
        while !cab.ends_with(b"\r\n\r\n") {
            cliente.read_exact(&mut byte).unwrap();
            cab.push(byte[0]);
        }
        let texto = String::from_utf8(cab).unwrap();
        let alvo = texto.split_whitespace().nth(1).unwrap().to_owned();
        if let Some((usuario, senha)) = credenciais {
            let esperado = format!(
                "Proxy-Authorization: Basic {}",
                base64(format!("{usuario}:{senha}").as_bytes())
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
    porta
}

fn socks_para(porta: u16, user: Option<&str>, password: Option<&str>) -> Proxy {
    Proxy::Socks5 {
        host: "127.0.0.1".into(),
        port: porta,
        user: user.map(str::to_owned),
        password: password.map(str::to_owned),
    }
}

fn http_para(porta: u16, user: Option<&str>, password: Option<&str>) -> Proxy {
    Proxy::Http {
        host: "127.0.0.1".into(),
        port: porta,
        user: user.map(str::to_owned),
        password: password.map(str::to_owned),
    }
}

fn confere_eco(proxy: &Proxy, destino: u16) {
    let mut tcp = connect("127.0.0.1", destino, Some(proxy), CONNECT_TIMEOUT).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    tcp.write_all(b"ola").unwrap();
    let mut resposta = [0u8; 3];
    tcp.read_exact(&mut resposta).unwrap();
    assert_eq!(&resposta, b"ola");
}

#[test]
fn socks5_sem_autenticacao_leva_ate_o_destino() {
    let destino = eco();
    confere_eco(&socks_para(socks5(None), None, None), destino);
}

#[test]
fn socks5_com_usuario_e_senha() {
    let destino = eco();
    let porta = socks5(Some(("ana", "segredo")));
    confere_eco(&socks_para(porta, Some("ana"), Some("segredo")), destino);
}

#[test]
fn socks5_com_senha_errada_diz_o_motivo() {
    let porta = socks5(Some(("ana", "segredo")));
    let proxy = socks_para(porta, Some("ana"), Some("errada"));
    let erro = connect("127.0.0.1", 9, Some(&proxy), CONNECT_TIMEOUT).unwrap_err();
    assert_eq!(erro.to_string(), "o proxy recusou o usuário ou a senha");
}

/// Um SOCKS5 sem autenticação que responde ao CONNECT com `codigo`.
fn socks5_que_responde(codigo: u8) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut cliente, _) = listener.accept().unwrap();
        let mut oferta = [0u8; 2];
        cliente.read_exact(&mut oferta).unwrap();
        let mut metodos = vec![0u8; usize::from(oferta[1])];
        cliente.read_exact(&mut metodos).unwrap();
        cliente.write_all(&[5, 0]).unwrap();
        // O pedido: 4 bytes de cabeçalho, o nome (tamanho + bytes) e a porta.
        let mut cab = [0u8; 5];
        cliente.read_exact(&mut cab).unwrap();
        let mut resto = vec![0u8; usize::from(cab[4]) + 2];
        cliente.read_exact(&mut resto).unwrap();
        cliente
            .write_all(&[5, codigo, 0, 1, 0, 0, 0, 0, 0, 0])
            .unwrap();
    });
    porta
}

#[test]
fn socks5_recusado_traz_o_motivo_do_codigo() {
    for (codigo, esperado) in [
        (3, "o proxy recusou a conexão (código 3: rede inalcançável)"),
        (4, "o proxy recusou a conexão (código 4: host inalcançável)"),
        (
            5,
            "o proxy recusou a conexão (código 5: conexão recusada pelo destino)",
        ),
        (6, "o proxy recusou a conexão (código 6: tempo esgotado)"),
        // Um código que a RFC não define fica só o número.
        (99, "o proxy recusou a conexão (código 99)"),
    ] {
        let porta = socks5_que_responde(codigo);
        let proxy = socks_para(porta, None, None);
        let erro = connect("10.42.0.2", 22, Some(&proxy), CONNECT_TIMEOUT).unwrap_err();
        assert_eq!(erro.to_string(), esperado);
    }
}

#[test]
fn http_connect_leva_ate_o_destino() {
    let destino = eco();
    confere_eco(&http_para(http(None), None, None), destino);
}

#[test]
fn http_connect_com_credenciais() {
    let destino = eco();
    let porta = http(Some(("ana", "segredo")));
    confere_eco(&http_para(porta, Some("ana"), Some("segredo")), destino);
}

#[test]
fn http_connect_pede_autenticacao() {
    let porta = http(Some(("ana", "segredo")));
    let erro = connect(
        "127.0.0.1",
        9,
        Some(&http_para(porta, None, None)),
        CONNECT_TIMEOUT,
    )
    .unwrap_err();
    assert_eq!(erro.to_string(), "o proxy pediu autenticação");
}

#[test]
fn proxy_que_nao_responde_diz_qual_proxy() {
    let porta = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let erro = connect(
        "127.0.0.1",
        9,
        Some(&socks_para(porta, None, None)),
        CONNECT_TIMEOUT,
    )
    .unwrap_err();
    assert_eq!(erro.to_string(), "proxy 127.0.0.1: conexão recusada");
}
