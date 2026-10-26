//! IA (`tomni_ui::ai`): uma pergunta, a resposta chegando aos pedaços num `MarkdownView` e um
//! botão para parar. `cargo run --example ia --features ai,markdown`.
//!
//! Sem configuração, a tela fala com um servidor falso (`ai::test_util::FakeAi`) que responde um
//! texto fixo, devagar. Para falar com um provedor de verdade, aponte as variáveis:
//! `TOMNI_UI_AI_URL` (até o `/v1`, por exemplo `http://localhost:11434/v1` no Ollama),
//! `TOMNI_UI_AI_MODEL` e, se preciso, `TOMNI_UI_AI_DIALECT=anthropic`.
//!
//! A thread do pedido grava na `Live` (que acorda a tela) e a tela lê como um signal. A lib só
//! fala com o provedor: quem redige segredos, limpa o texto e decide o que fazer com a resposta
//! é o app. Aqui o botão "Copiar" do bloco de código é o único destino da resposta.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use tomni_ui::ai::test_util::FakeAi;
use tomni_ui::ai::{Dialect, Endpoint, Event, Message, Request, StreamHandle, stream};
use tomni_ui::files::Live;
use tomni_ui::markdown::MarkdownView;
use tomni_ui::prelude::*;

/// A resposta que o servidor falso dá, em pedaços.
const RESPOSTA_FALSA: [&str; 4] = [
    "Para ver as portas em escuta:\n\n",
    "```sh\nss -tlnp\n```\n\n",
    "A coluna `Local Address` mostra a porta ",
    "e o programa que a abriu.",
];

/// O que a tela mostra da resposta.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resposta {
    /// O texto recebido até agora.
    pub texto: String,
    /// Se o pedido está em andamento.
    pub rodando: bool,
    /// O erro, já em português.
    pub erro: Option<String>,
}

/// Para onde o pedido vai: o provedor das variáveis, ou o servidor falso.
fn endpoint(falso: &FakeAi) -> Endpoint {
    match std::env::var("TOMNI_UI_AI_URL") {
        Ok(url) if !url.is_empty() => {
            let dialeto = match std::env::var("TOMNI_UI_AI_DIALECT").as_deref() {
                Ok("anthropic") => Dialect::Anthropic,
                _ => Dialect::OpenAi,
            };
            let modelo = std::env::var("TOMNI_UI_AI_MODEL").unwrap_or_default();
            Endpoint::new(dialeto, url, modelo)
        }
        _ => Endpoint::new(Dialect::OpenAi, falso.url(), "falso"),
    }
}

/// A tela: o campo da pergunta, "Perguntar" e "Parar", e a resposta.
pub fn perguntar() -> impl View {
    let falso = Rc::new(FakeAi::novo());
    falso
        .responder(&RESPOSTA_FALSA)
        .com_pausa(Duration::from_millis(40));
    let ponto = endpoint(&falso);
    let pergunta = signal(String::from("Como vejo as portas abertas?"));
    let resposta = Live::new(Resposta::default());
    let pedido: Rc<RefCell<Option<StreamHandle>>> = Rc::default();

    let perguntar = {
        let (resposta, pedido, falso) = (resposta.clone(), Rc::clone(&pedido), Rc::clone(&falso));
        move || {
            // O servidor falso vive enquanto a tela viver: o botão o guarda.
            let _ = &falso;
            if let Some(antigo) = pedido.borrow_mut().take() {
                antigo.cancel();
            }
            resposta.set(Resposta {
                rodando: true,
                ..Resposta::default()
            });
            let pergunta = pergunta.get_untracked();
            let destino = resposta.clone();
            let novo = stream(
                ponto.clone(),
                Request {
                    system: "Responda em português, com um comando por vez.".into(),
                    messages: vec![Message::user(pergunta)],
                    ..Request::default()
                },
                move |evento| match evento {
                    Event::Text(pedaco) => destino.update(|r| r.texto.push_str(&pedaco)),
                    Event::Reasoning(_) => {}
                    Event::Done(_) => destino.update(|r| r.rodando = false),
                    Event::Error(erro) => destino.update(|r| {
                        r.rodando = false;
                        r.erro = Some(erro.message);
                    }),
                },
            );
            *pedido.borrow_mut() = Some(novo);
        }
    };
    let parar = {
        let (resposta, pedido) = (resposta.clone(), Rc::clone(&pedido));
        move || {
            if let Some(atual) = pedido.borrow_mut().take() {
                atual.cancel();
            }
            resposta.update(|r| r.rodando = false);
        }
    };

    let rodando = {
        let resposta = resposta.clone();
        move || resposta.with(|r| r.rodando)
    };
    let texto = {
        let resposta = resposta.clone();
        move || resposta.with(|r| r.texto.clone())
    };
    let erro = {
        let resposta = resposta.clone();
        move || resposta.with(|r| r.erro.clone().unwrap_or_default())
    };
    let tem_erro = {
        let resposta = resposta.clone();
        move || resposta.with(|r| r.erro.is_some())
    };
    Form::new("IA").child(
        Column::new()
            .gap(12)
            .padding(16)
            .child(
                Edit::bind(pergunta)
                    .id("pergunta")
                    .placeholder("Pergunte alguma coisa")
                    .height(44),
            )
            .child(
                Row::new()
                    .gap(8)
                    .child(
                        Button::new("Perguntar")
                            .id("perguntar")
                            .height(44)
                            .disabled(rodando.clone())
                            .on_click(perguntar),
                    )
                    .child(
                        Button::new("Parar")
                            .id("parar")
                            .height(44)
                            .disabled({
                                let rodando = rodando.clone();
                                move || !rodando()
                            })
                            .on_click(parar),
                    ),
            )
            .child(Text::new(erro).id("erro").secondary().visible(tem_erro))
            .child(
                ScrollViewer::new()
                    .id("rolagem")
                    .grow(1)
                    .child(MarkdownView::new(texto).id("resposta")),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(perguntar).size(720, 520).run()
}
