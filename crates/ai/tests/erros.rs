//! Os erros em português, a lista de modelos e o proxy.
#![allow(missing_docs)]

mod comum;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use comum::{pergunta, ponto, rodar, ultimo};
use tomni_ui_ai::test_util::FakeAi;
use tomni_ui_ai::{AiError, AiErrorKind, Dialect, Endpoint, Event, models};
use tomni_ui_net::Proxy;

fn erro_de(ia: &FakeAi) -> AiError {
    match ultimo(&rodar(ponto(ia, Dialect::OpenAi), pergunta("x"))) {
        Event::Error(e) => e,
        outro => panic!("devia dar erro: {outro:?}"),
    }
}

#[test]
fn erros_http_em_portugues() {
    let casos: [(u16, &str, AiErrorKind, &str); 8] = [
        (
            401,
            r#"{"error":{"message":"Incorrect API key"}}"#,
            AiErrorKind::Auth,
            "O provedor recusou a chave (HTTP 401): Incorrect API key.",
        ),
        (
            403,
            r#"{"message":"Sem permissão"}"#,
            AiErrorKind::Auth,
            "O provedor recusou a chave (HTTP 403): Sem permissão.",
        ),
        (
            404,
            r#"{"error":{"message":"model 'x' not found"}}"#,
            AiErrorKind::NotFound,
            "O provedor não conhece este endereço ou este modelo (HTTP 404): model 'x' not found.",
        ),
        (
            429,
            "{}",
            AiErrorKind::RateLimit,
            "O provedor limitou os pedidos; tente de novo em instantes.",
        ),
        (
            503,
            "",
            AiErrorKind::Overloaded,
            "O provedor está sobrecarregado (HTTP 503); tente de novo em instantes.",
        ),
        (
            529,
            "",
            AiErrorKind::Overloaded,
            "O provedor está sobrecarregado (HTTP 529); tente de novo em instantes.",
        ),
        (
            500,
            "falhou   feio\ncom quebra",
            AiErrorKind::Server,
            "O provedor respondeu HTTP 500: falhou feio com quebra.",
        ),
        (
            400,
            "",
            AiErrorKind::Server,
            "O provedor respondeu HTTP 400.",
        ),
    ];
    for (status, corpo, tipo, mensagem) in casos {
        let ia = FakeAi::novo();
        ia.erro(status, corpo);
        let erro = erro_de(&ia);
        assert_eq!(erro.kind, tipo, "status {status}");
        assert_eq!(erro.status, Some(status));
        assert_eq!(erro.message, mensagem, "status {status}");
    }
}

#[test]
fn retry_after_na_mensagem() {
    let ia = FakeAi::novo();
    ia.erro(429, "{}").retry_after(30);
    let erro = erro_de(&ia);
    assert_eq!(erro.kind, AiErrorKind::RateLimit);
    assert_eq!(
        erro.message,
        "O provedor limitou os pedidos; tente de novo em 30 s."
    );
}

#[test]
fn conexao_recusada() {
    // Uma porta que acabou de ser liberada: ninguém atende.
    let porta = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let ponto = Endpoint::new(Dialect::OpenAi, format!("http://127.0.0.1:{porta}/v1"), "m");
    let Event::Error(erro) = ultimo(&rodar(ponto, pergunta("x"))) else {
        panic!("devia dar erro");
    };
    assert_eq!(erro.kind, AiErrorKind::Connect);
    assert_eq!(
        erro.message,
        format!("Nada respondeu em 127.0.0.1:{porta}.")
    );
}

#[test]
fn nome_que_nao_existe() {
    let ponto = Endpoint::new(Dialect::OpenAi, "http://nao-existe.invalid/v1", "m");
    let Event::Error(erro) = ultimo(&rodar(ponto, pergunta("x"))) else {
        panic!("devia dar erro");
    };
    assert_eq!(erro.kind, AiErrorKind::Connect);
    assert!(
        erro.message
            .starts_with("Não achei o endereço nao-existe.invalid")
            || erro
                .message
                .starts_with("Nada respondeu em nao-existe.invalid"),
        "{}",
        erro.message
    );
}

#[test]
fn modelos_openai() {
    let ia = FakeAi::novo();
    ia.modelos(&["llama3", "qwen2.5-coder"]);
    let lista = models(&ponto(&ia, Dialect::OpenAi)).unwrap();
    let ids: Vec<&str> = lista.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["llama3", "qwen2.5-coder"]);
    assert_eq!(ia.pedidos()[0].caminho, "/v1/models");
    assert_eq!(ia.pedidos()[0].metodo, "GET");
}

#[test]
fn modelos_anthropic_paginados() {
    let ia = FakeAi::novo();
    ia.modelos(&["a", "b", "c", "d", "e"]).modelos_por_pagina(2);
    let lista = models(&ponto(&ia, Dialect::Anthropic)).unwrap();
    let ids: Vec<&str> = lista.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["a", "b", "c", "d", "e"]);
    assert_eq!(lista[0].name.as_deref(), Some("Modelo a"));
    assert_eq!(ia.pedidos().len(), 3);
    assert!(ia.pedidos()[1].caminho.contains("after_id=b"));
}

#[test]
fn modelos_com_chave_errada() {
    let ia = FakeAi::novo();
    ia.modelos(&["a"]).exigir_chave("certa");
    let erro = models(&ponto(&ia, Dialect::OpenAi)).unwrap_err();
    assert_eq!(erro.kind, AiErrorKind::Auth);
    assert!(erro.message.contains("Chave inválida"), "{erro}");
}

/// Um proxy HTTP CONNECT de teste: aceita uma conexão, responde 200 e liga ao destino.
fn proxy_connect() -> (u16, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let porta = listener.local_addr().unwrap().port();
    let tarefa = thread::spawn(move || {
        let (mut cliente, _) = listener.accept().unwrap();
        let mut cabecalho = Vec::new();
        let mut byte = [0u8; 1];
        while !cabecalho.ends_with(b"\r\n\r\n") {
            cliente.read_exact(&mut byte).unwrap();
            cabecalho.push(byte[0]);
        }
        let texto = String::from_utf8_lossy(&cabecalho).into_owned();
        let alvo = texto.split_whitespace().nth(1).unwrap().to_owned();
        let destino = std::net::TcpStream::connect(&alvo).unwrap();
        cliente
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .unwrap();
        let (mut c1, mut d1) = (cliente.try_clone().unwrap(), destino.try_clone().unwrap());
        thread::spawn(move || {
            let _ = std::io::copy(&mut c1, &mut d1);
        });
        let (mut c2, mut d2) = (cliente, destino);
        let _ = std::io::copy(&mut d2, &mut c2);
        alvo
    });
    (porta, tarefa)
}

#[test]
fn proxy_http_connect() {
    let ia = FakeAi::novo();
    ia.responder(&["pelo proxy"]);
    let (porta, tarefa) = proxy_connect();
    let mut ponto = ponto(&ia, Dialect::OpenAi);
    ponto.proxy = Some(Proxy::Http {
        host: "127.0.0.1".into(),
        port: porta,
        user: None,
        password: None,
    });
    let coleta = rodar(ponto, pergunta("x"));
    assert_eq!(coleta.texto(), "pelo proxy");
    let alvo = tarefa.join().unwrap();
    assert_eq!(alvo, format!("127.0.0.1:{}", ia.porta()));
}
