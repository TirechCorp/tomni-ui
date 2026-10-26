//! Teste contra um Ollama de verdade. Fora do CI: `TOMNI_UI_OLLAMA=http://localhost:11434/v1
//! cargo test -p tomni-ui-ai --test ollama_real -- --ignored`.
#![allow(missing_docs)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tomni_ui_ai::{Dialect, Endpoint, Event, Message, Request, models, stream};

#[test]
#[ignore = "precisa de um Ollama: defina TOMNI_UI_OLLAMA"]
fn lista_os_modelos_e_pede_uma_resposta_curta() {
    let Ok(url) = std::env::var("TOMNI_UI_OLLAMA") else {
        panic!("defina TOMNI_UI_OLLAMA com a URL base, por exemplo http://localhost:11434/v1");
    };
    let mut ponto = Endpoint::new(Dialect::OpenAi, url, "");
    let lista = models(&ponto).expect("a lista de modelos");
    assert!(!lista.is_empty(), "o Ollama não tem nenhum modelo baixado");
    ponto.model = lista[0].id.clone();

    let eventos = Arc::new(Mutex::new(Vec::new()));
    let fila = eventos.clone();
    let pedido = Request {
        system: "Responda com uma palavra só.".into(),
        messages: vec![Message::user("Diga oi.")],
        max_tokens: Some(64),
        ..Request::default()
    };
    let handle = stream(ponto, pedido, move |e| fila.lock().unwrap().push(e));
    let limite = Instant::now() + Duration::from_secs(300);
    while !handle.finished() {
        assert!(Instant::now() < limite, "o Ollama demorou demais");
        std::thread::sleep(Duration::from_millis(50));
    }
    let eventos = eventos.lock().unwrap();
    assert!(
        matches!(eventos.last(), Some(Event::Done(_))),
        "terminou assim: {:?}",
        eventos.last()
    );
}
