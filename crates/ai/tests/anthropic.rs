//! O dialeto da Anthropic contra o `FakeAi`.
#![allow(missing_docs)]

mod comum;

use comum::{pergunta, ponto, rodar, ultimo};
use tomni_ui_ai::test_util::FakeAi;
use tomni_ui_ai::{
    AiErrorKind, Dialect, Event, Finish, FinishReason, Message, Request, Usage, Zeroizing,
};

#[test]
fn anthropic_texto_ping_e_tipos_desconhecidos() {
    let ia = FakeAi::novo();
    ia.responder(&["Olá, ", "mundo"]).com_eventos_extras();
    let coleta = rodar(ponto(&ia, Dialect::Anthropic), pergunta("oi"));
    assert_eq!(coleta.texto(), "Olá, mundo");
    assert!(matches!(ultimo(&coleta), Event::Done(_)));
    assert_eq!(ia.pedidos()[0].caminho, "/v1/messages");
}

#[test]
fn anthropic_thinking_vira_raciocinio() {
    let ia = FakeAi::novo();
    ia.responder(&["pronto"]).raciocinio("pensando alto");
    let coleta = rodar(ponto(&ia, Dialect::Anthropic), pergunta("x"));
    assert_eq!(coleta.raciocinio(), "pensando alto");
    assert_eq!(coleta.texto(), "pronto");
}

#[test]
fn anthropic_cabecalhos_e_versao() {
    let ia = FakeAi::novo();
    ia.responder(&["ok"]);
    let mut com_chave = ponto(&ia, Dialect::Anthropic);
    com_chave.api_key = Some(Zeroizing::new("sk-ant-segredo".into()));
    rodar(com_chave, pergunta("x"));
    let pedido = &ia.pedidos()[0];
    assert_eq!(pedido.cabecalho("x-api-key"), Some("sk-ant-segredo"));
    assert_eq!(pedido.cabecalho("anthropic-version"), Some("2023-06-01"));
    assert_eq!(pedido.cabecalho("authorization"), None);
    assert_eq!(pedido.cabecalho("content-type"), Some("application/json"));
}

#[test]
fn anthropic_max_tokens_padrao() {
    let ia = FakeAi::novo();
    ia.responder(&["ok"]);
    rodar(ponto(&ia, Dialect::Anthropic), pergunta("x"));
    let corpo = ia.pedidos()[0].json().unwrap();
    assert_eq!(corpo["max_tokens"], 4096);
    assert_eq!(corpo["system"], "Seja breve.");
    assert_eq!(corpo["stream"], true);

    let pedido = Request {
        max_tokens: Some(500),
        system: String::new(),
        ..pergunta("x")
    };
    rodar(ponto(&ia, Dialect::Anthropic), pedido);
    let corpo = ia.pedidos()[1].json().unwrap();
    assert_eq!(corpo["max_tokens"], 500);
    assert!(corpo.get("system").is_none(), "system vazio não vai");
}

#[test]
fn anthropic_junta_papeis_repetidos() {
    let ia = FakeAi::novo();
    ia.responder(&["ok"]);
    let pedido = Request {
        messages: vec![
            Message::user("um"),
            Message::user("dois"),
            Message::assistant("três"),
            Message::user("quatro"),
        ],
        ..Request::default()
    };
    rodar(ponto(&ia, Dialect::Anthropic), pedido);
    let corpo = ia.pedidos()[0].json().unwrap();
    let mensagens = corpo["messages"].as_array().unwrap();
    assert_eq!(mensagens.len(), 3);
    assert_eq!(mensagens[0]["content"], "um\n\ndois");
    assert_eq!(mensagens[1]["role"], "assistant");
}

#[test]
fn anthropic_primeira_mensagem_e_do_usuario() {
    let ia = FakeAi::novo();
    ia.responder(&["ok"]);
    let pedido = Request {
        messages: vec![Message::assistant("eu falei primeiro")],
        ..Request::default()
    };
    rodar(ponto(&ia, Dialect::Anthropic), pedido);
    let corpo = ia.pedidos()[0].json().unwrap();
    assert_eq!(corpo["messages"][0]["role"], "user");
    assert_eq!(corpo["messages"][0]["content"], "(continuação)");
    assert_eq!(corpo["messages"][1]["role"], "assistant");
}

#[test]
fn anthropic_overloaded() {
    let ia = FakeAi::novo();
    ia.erro(
        529,
        r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
    );
    let coleta = rodar(ponto(&ia, Dialect::Anthropic), pergunta("x"));
    let Event::Error(erro) = ultimo(&coleta) else {
        panic!("devia dar erro");
    };
    assert_eq!(erro.kind, AiErrorKind::Overloaded);
    assert_eq!(erro.status, Some(529));

    // E o mesmo erro no meio do streaming.
    let ia = FakeAi::novo();
    ia.responder(&["a"]).erro_no_meio("Overloaded");
    let coleta = rodar(ponto(&ia, Dialect::Anthropic), pergunta("x"));
    let Event::Error(erro) = ultimo(&coleta) else {
        panic!("devia dar erro");
    };
    assert_eq!(erro.kind, AiErrorKind::Overloaded);
}

#[test]
fn anthropic_uso_de_entrada_e_saida() {
    let ia = FakeAi::novo();
    ia.responder(&["ok"]);
    let coleta = rodar(ponto(&ia, Dialect::Anthropic), pergunta("x"));
    assert_eq!(
        ultimo(&coleta),
        Event::Done(Finish {
            reason: FinishReason::Stop,
            usage: Some(Usage {
                input: Some(12),
                output: Some(5)
            }),
        })
    );
}

#[test]
fn anthropic_resposta_sem_streaming() {
    let ia = FakeAi::novo();
    ia.sem_streaming("de uma vez");
    let coleta = rodar(ponto(&ia, Dialect::Anthropic), pergunta("x"));
    assert_eq!(coleta.texto(), "de uma vez");
    assert!(matches!(ultimo(&coleta), Event::Done(_)));
}

#[test]
fn anthropic_fechou_no_meio_com_texto() {
    let ia = FakeAi::novo();
    ia.responder(&["a", "b"]).fechar_no_meio();
    let coleta = rodar(ponto(&ia, Dialect::Anthropic), pergunta("x"));
    assert_eq!(coleta.texto(), "a");
    assert_eq!(
        ultimo(&coleta),
        Event::Done(Finish {
            reason: FinishReason::Other("fim sem aviso".into()),
            usage: Some(Usage {
                input: Some(12),
                output: None
            }),
        })
    );
}

#[test]
fn anthropic_max_tokens_vira_length() {
    let ia = FakeAi::novo();
    ia.responder(&["cortad"]).parar_por_tamanho();
    let coleta = rodar(ponto(&ia, Dialect::Anthropic), pergunta("x"));
    assert!(matches!(
        ultimo(&coleta),
        Event::Done(Finish {
            reason: FinishReason::Length,
            ..
        })
    ));
}
