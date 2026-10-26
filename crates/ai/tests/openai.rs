//! O dialeto compatível com OpenAI contra o `FakeAi`.
#![allow(missing_docs)]

mod comum;

use std::time::Duration;

use comum::{Coleta, pergunta, ponto, rodar, ultimo};
use tomni_ui_ai::test_util::FakeAi;
use tomni_ui_ai::{
    AiErrorKind, Dialect, Event, Finish, FinishReason, Message, Request, Usage, Zeroizing,
};

fn erro_de(coleta: &Coleta) -> tomni_ui_ai::AiError {
    match ultimo(coleta) {
        Event::Error(e) => e,
        outro => panic!("devia terminar em erro: {outro:?}"),
    }
}

#[test]
fn openai_texto_em_partes_e_done() {
    let ia = FakeAi::novo();
    ia.responder(&["Olá, ", "mundo", "!"]);
    let coleta = rodar(ponto(&ia, Dialect::OpenAi), pergunta("diga oi"));
    assert_eq!(coleta.texto(), "Olá, mundo!");
    let eventos = coleta.eventos();
    assert_eq!(eventos.len(), 4, "três partes e o fim: {eventos:?}");
    assert_eq!(
        eventos[3],
        Event::Done(Finish {
            reason: FinishReason::Stop,
            usage: Some(Usage {
                input: Some(3),
                output: Some(5)
            }),
        })
    );
    let pedido = &ia.pedidos()[0];
    assert_eq!(pedido.caminho, "/v1/chat/completions");
    let corpo = pedido.json().unwrap();
    assert_eq!(corpo["stream"], true);
    assert_eq!(corpo["model"], "modelo-de-teste");
    assert_eq!(corpo["messages"][0]["role"], "system");
    assert_eq!(corpo["messages"][1]["content"], "diga oi");
    assert!(corpo.get("stream_options").is_none());
}

#[test]
fn openai_raciocinio_separado() {
    let ia = FakeAi::novo();
    ia.responder(&["resposta"]).raciocinio("hum, deixa ver");
    let coleta = rodar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    assert_eq!(coleta.raciocinio(), "hum, deixa ver");
    assert_eq!(coleta.texto(), "resposta");
}

#[test]
fn openai_finish_length() {
    let ia = FakeAi::novo();
    ia.responder(&["cortad"]).parar_por_tamanho();
    let coleta = rodar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    assert!(matches!(
        ultimo(&coleta),
        Event::Done(Finish {
            reason: FinishReason::Length,
            ..
        })
    ));
}

#[test]
fn openai_erro_no_meio_do_streaming() {
    let ia = FakeAi::novo();
    ia.responder(&["pedaço"])
        .erro_no_meio("Provider returned error");
    let coleta = rodar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    assert_eq!(coleta.texto(), "pedaço");
    let erro = erro_de(&coleta);
    assert_eq!(erro.kind, AiErrorKind::Server);
    assert!(erro.message.contains("Provider returned error"), "{erro}");
    // Nada depois do erro.
    assert!(matches!(coleta.eventos().last(), Some(Event::Error(_))));
}

#[test]
fn openai_resposta_sem_streaming() {
    let ia = FakeAi::novo();
    ia.sem_streaming("Tudo de uma vez");
    let coleta = rodar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    let eventos = coleta.eventos();
    assert_eq!(eventos[0], Event::Text("Tudo de uma vez".into()));
    assert!(matches!(eventos[1], Event::Done(_)));
    assert_eq!(eventos.len(), 2);
}

#[test]
fn openai_fechou_sem_done_com_texto() {
    let ia = FakeAi::novo();
    ia.responder(&["a", "b"]).fechar_no_meio();
    let coleta = rodar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    assert_eq!(coleta.texto(), "a");
    assert_eq!(
        ultimo(&coleta),
        Event::Done(Finish {
            reason: FinishReason::Other("fim sem aviso".into()),
            usage: None
        })
    );
}

#[test]
fn openai_fechou_sem_nada_e_erro() {
    let ia = FakeAi::novo();
    ia.responder(&[]).fechar_no_meio();
    let coleta = rodar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    assert_eq!(erro_de(&coleta).kind, AiErrorKind::Protocol);
}

#[test]
fn openai_bearer_so_com_chave() {
    let ia = FakeAi::novo();
    ia.responder(&["ok"]);
    rodar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    assert_eq!(ia.pedidos()[0].cabecalho("authorization"), None);

    let mut com_chave = ponto(&ia, Dialect::OpenAi);
    com_chave.api_key = Some(Zeroizing::new("sk-segredo".into()));
    com_chave.headers = vec![("X-Title".into(), "Teste".into())];
    rodar(com_chave, pergunta("x"));
    let pedido = &ia.pedidos()[1];
    assert_eq!(pedido.cabecalho("authorization"), Some("Bearer sk-segredo"));
    assert_eq!(pedido.cabecalho("x-title"), Some("Teste"));
}

#[test]
fn openai_sem_max_tokens_quando_none() {
    let ia = FakeAi::novo();
    ia.responder(&["ok"]);
    rodar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    let corpo = ia.pedidos()[0].json().unwrap();
    assert!(corpo.get("max_tokens").is_none());
    assert!(corpo.get("temperature").is_none());

    let pedido = Request {
        max_tokens: Some(300),
        temperature: Some(0.5),
        ..pergunta("x")
    };
    rodar(ponto(&ia, Dialect::OpenAi), pedido);
    let corpo = ia.pedidos()[1].json().unwrap();
    assert_eq!(corpo["max_tokens"], 300);
    assert_eq!(corpo["temperature"], 0.5);
}

#[test]
fn openai_comentarios_e_eventos_extras_sao_ignorados() {
    let ia = FakeAi::novo();
    ia.responder(&["a", "b"]).com_eventos_extras();
    let coleta = rodar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    assert_eq!(coleta.texto(), "ab");
    assert!(matches!(ultimo(&coleta), Event::Done(_)));
}

#[test]
fn openai_conversa_com_varias_mensagens() {
    let ia = FakeAi::novo();
    ia.responder(&["ok"]);
    let pedido = Request {
        system: String::new(),
        messages: vec![
            Message::user("um"),
            Message::assistant("dois"),
            Message::user("três"),
        ],
        ..Request::default()
    };
    rodar(ponto(&ia, Dialect::OpenAi), pedido);
    let corpo = ia.pedidos()[0].json().unwrap();
    let papeis: Vec<&str> = corpo["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    // Sem prompt de sistema, nada de mensagem `system`.
    assert_eq!(papeis, ["user", "assistant", "user"]);
}

#[test]
fn cancelar_para_os_eventos() {
    let ia = FakeAi::novo();
    let partes: Vec<String> = (0..200).map(|i| format!("p{i} ")).collect();
    let partes: Vec<&str> = partes.iter().map(String::as_str).collect();
    ia.responder(&partes).com_pausa(Duration::from_millis(5));
    let (pedido, coleta) = comum::iniciar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    comum::esperar(|| coleta.eventos().len() >= 3);
    pedido.cancel();
    let ao_cancelar = coleta.eventos().len();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        coleta.eventos().len(),
        ao_cancelar,
        "chegou evento depois do cancel"
    );
    // A thread termina quando o servidor fecha (ou na próxima leitura).
    comum::esperar(|| pedido.finished());
    assert_eq!(coleta.eventos().len(), ao_cancelar);
}

#[test]
fn finished_depois_do_done() {
    let ia = FakeAi::novo();
    ia.responder(&["ok"]);
    let (pedido, coleta) = comum::iniciar(ponto(&ia, Dialect::OpenAi), pergunta("x"));
    comum::esperar(|| pedido.finished());
    assert!(matches!(ultimo(&coleta), Event::Done(_)));
}

#[test]
fn chave_nao_aparece_no_debug() {
    let mut ponto = tomni_ui_ai::Endpoint::new(Dialect::OpenAi, "http://x/v1", "m");
    ponto.api_key = Some(Zeroizing::new("sk-segredo-123".into()));
    ponto.headers = vec![("Authorization".into(), "Bearer outro-segredo".into())];
    let texto = format!("{ponto:?}");
    assert!(!texto.contains("sk-segredo-123"), "{texto}");
    assert!(!texto.contains("outro-segredo"), "{texto}");
}
