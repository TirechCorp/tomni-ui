//! Ajudas dos testes: coletar os eventos de um pedido e esperar o fim.
#![allow(dead_code, missing_docs)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tomni_ui_ai::test_util::FakeAi;
use tomni_ui_ai::{Dialect, Endpoint, Event, Message, Request, StreamHandle, stream};

pub fn ponto(ia: &FakeAi, dialeto: Dialect) -> Endpoint {
    Endpoint::new(dialeto, ia.url(), "modelo-de-teste")
}

pub fn pergunta(texto: &str) -> Request {
    Request {
        system: "Seja breve.".into(),
        messages: vec![Message::user(texto)],
        ..Request::default()
    }
}

/// Os eventos que chegaram, para o teste ler enquanto o pedido corre.
#[derive(Clone, Default)]
pub struct Coleta(Arc<Mutex<Vec<Event>>>);

impl Coleta {
    pub fn eventos(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }

    /// O texto da resposta (só os `Text`).
    pub fn texto(&self) -> String {
        self.eventos()
            .into_iter()
            .filter_map(|e| match e {
                Event::Text(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    pub fn raciocinio(&self) -> String {
        self.eventos()
            .into_iter()
            .filter_map(|e| match e {
                Event::Reasoning(t) => Some(t),
                _ => None,
            })
            .collect()
    }
}

pub fn iniciar(ponto: Endpoint, pedido: Request) -> (StreamHandle, Coleta) {
    let coleta = Coleta::default();
    let fila = coleta.0.clone();
    let pedido = stream(ponto, pedido, move |e| fila.lock().unwrap().push(e));
    (pedido, coleta)
}

/// Roda o pedido até a thread terminar.
pub fn rodar(ponto: Endpoint, pedido: Request) -> Coleta {
    let (handle, coleta) = iniciar(ponto, pedido);
    esperar(|| handle.finished());
    coleta
}

pub fn esperar(mut pronto: impl FnMut() -> bool) {
    let limite = Instant::now() + Duration::from_secs(10);
    while !pronto() {
        assert!(Instant::now() < limite, "o teste esperou demais");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// O último evento, que deve ser o fim.
pub fn ultimo(coleta: &Coleta) -> Event {
    coleta.eventos().pop().expect("nenhum evento chegou")
}
