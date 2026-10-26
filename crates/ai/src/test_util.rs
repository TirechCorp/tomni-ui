//! O servidor falso dos testes: fala os dois dialetos em `127.0.0.1`, toca um roteiro por
//! pedido e guarda o que recebeu.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

/// Um pedido que o servidor recebeu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PedidoFalso {
    /// O método (`POST`, `GET`).
    pub metodo: String,
    /// O caminho, com a query (`/v1/chat/completions`).
    pub caminho: String,
    /// Os cabeçalhos, com os nomes em minúsculas, na ordem.
    pub cabecalhos: Vec<(String, String)>,
    /// O corpo.
    pub corpo: String,
}

impl PedidoFalso {
    /// O valor do cabeçalho `nome` (sem distinguir maiúsculas).
    pub fn cabecalho(&self, nome: &str) -> Option<&str> {
        self.cabecalhos
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(nome))
            .map(|(_, v)| v.as_str())
    }

    /// O corpo como JSON, se for um.
    pub fn json(&self) -> Option<Value> {
        serde_json::from_str(&self.corpo).ok()
    }
}

#[derive(Clone, Debug)]
enum Tipo {
    Partes(Vec<String>),
    Erro { status: u16, corpo: String },
    Inteiro(String),
}

#[derive(Clone, Debug)]
struct Roteiro {
    tipo: Tipo,
    pausa: Duration,
    fechar_no_meio: bool,
    extras: bool,
    raciocinio: Option<String>,
    erro_no_meio: Option<String>,
    retry_after: Option<u64>,
    por_tamanho: bool,
}

impl Roteiro {
    fn novo(tipo: Tipo) -> Self {
        Self {
            tipo,
            pausa: Duration::ZERO,
            fechar_no_meio: false,
            extras: false,
            raciocinio: None,
            erro_no_meio: None,
            retry_after: None,
            por_tamanho: false,
        }
    }
}

#[derive(Default)]
struct Estado {
    roteiros: VecDeque<Roteiro>,
    pedidos: Vec<PedidoFalso>,
    modelos: Vec<String>,
    por_pagina: Option<usize>,
    chave: Option<String>,
}

/// Um servidor de IA falso, que responde aos dois dialetos (`/chat/completions`, `/messages`,
/// `/models`) em `127.0.0.1`. Termina quando sai de escopo.
///
/// Cada `responder`, `erro` ou `sem_streaming` acrescenta o roteiro do **próximo** pedido; os
/// modificadores (`com_pausa`, `raciocinio`…) valem para o último roteiro acrescentado. Os
/// roteiros são tocados na ordem e o último se repete; sem nenhum, a resposta é `ok`.
///
/// ```
/// use tomni_ui_ai::test_util::FakeAi;
///
/// let ia = FakeAi::novo();
/// ia.responder(&["Olá, ", "mundo"]).com_pausa(std::time::Duration::from_millis(1));
/// assert!(ia.url().starts_with("http://127.0.0.1:"));
/// ```
pub struct FakeAi {
    porta: u16,
    estado: Arc<Mutex<Estado>>,
    parar: Arc<AtomicBool>,
}

impl FakeAi {
    /// Sobe o servidor numa porta livre.
    pub fn novo() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("o FakeAi não achou uma porta");
        let porta = listener.local_addr().expect("endereço do FakeAi").port();
        let estado = Arc::new(Mutex::new(Estado::default()));
        let parar = Arc::new(AtomicBool::new(false));
        {
            let estado = estado.clone();
            let parar = parar.clone();
            thread::spawn(move || {
                for conexao in listener.incoming() {
                    if parar.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(conexao) = conexao else { continue };
                    let estado = estado.clone();
                    thread::spawn(move || atender(conexao, &estado));
                }
            });
        }
        Self {
            porta,
            estado,
            parar,
        }
    }

    /// A URL base (`http://127.0.0.1:porta/v1`), a que vai no `Endpoint`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.porta)
    }

    /// A porta do servidor.
    pub fn porta(&self) -> u16 {
        self.porta
    }

    fn estado(&self) -> MutexGuard<'_, Estado> {
        self.estado.lock().expect("estado do FakeAi")
    }

    fn acrescentar(&self, tipo: Tipo) -> &Self {
        self.estado().roteiros.push_back(Roteiro::novo(tipo));
        self
    }

    fn ultimo(&self, f: impl FnOnce(&mut Roteiro)) -> &Self {
        {
            let mut estado = self.estado();
            if estado.roteiros.is_empty() {
                estado
                    .roteiros
                    .push_back(Roteiro::novo(Tipo::Partes(vec!["ok".into()])));
            }
            if let Some(ultimo) = estado.roteiros.back_mut() {
                f(ultimo);
            }
        }
        self
    }

    /// O próximo pedido recebe `partes`, uma por evento.
    pub fn responder(&self, partes: &[&str]) -> &Self {
        self.acrescentar(Tipo::Partes(
            partes.iter().map(|p| (*p).to_owned()).collect(),
        ))
    }

    /// O próximo pedido recebe um erro HTTP com este corpo.
    pub fn erro(&self, status: u16, corpo: &str) -> &Self {
        self.acrescentar(Tipo::Erro {
            status,
            corpo: corpo.to_owned(),
        })
    }

    /// O próximo pedido recebe a resposta inteira em JSON, como um servidor que ignora o
    /// `stream`.
    pub fn sem_streaming(&self, texto: &str) -> &Self {
        self.acrescentar(Tipo::Inteiro(texto.to_owned()))
    }

    /// Espera `pausa` entre os eventos do último roteiro.
    pub fn com_pausa(&self, pausa: Duration) -> &Self {
        self.ultimo(|r| r.pausa = pausa)
    }

    /// O último roteiro fecha a conexão na metade das partes, sem o fim.
    pub fn fechar_no_meio(&self) -> &Self {
        self.ultimo(|r| r.fechar_no_meio = true)
    }

    /// O último roteiro mistura comentários, `ping` e tipos de evento desconhecidos.
    pub fn com_eventos_extras(&self) -> &Self {
        self.ultimo(|r| r.extras = true)
    }

    /// O último roteiro manda este texto como "raciocínio" antes da resposta.
    pub fn raciocinio(&self, texto: &str) -> &Self {
        self.ultimo(|r| r.raciocinio = Some(texto.to_owned()))
    }

    /// O último roteiro manda um erro do provedor no meio do streaming, depois das partes.
    pub fn erro_no_meio(&self, mensagem: &str) -> &Self {
        self.ultimo(|r| r.erro_no_meio = Some(mensagem.to_owned()))
    }

    /// O último roteiro (um `erro`) manda `Retry-After: segundos`.
    pub fn retry_after(&self, segundos: u64) -> &Self {
        self.ultimo(|r| r.retry_after = Some(segundos))
    }

    /// O último roteiro termina por falta de tokens (`length` / `max_tokens`).
    pub fn parar_por_tamanho(&self) -> &Self {
        self.ultimo(|r| r.por_tamanho = true)
    }

    /// Os modelos que `GET /models` lista.
    pub fn modelos(&self, ids: &[&str]) -> &Self {
        self.estado().modelos = ids.iter().map(|i| (*i).to_owned()).collect();
        self
    }

    /// Quantos modelos cada página da lista da Anthropic traz (`has_more` e `last_id`).
    pub fn modelos_por_pagina(&self, n: usize) -> &Self {
        self.estado().por_pagina = Some(n);
        self
    }

    /// Passa a exigir esta chave (`Authorization: Bearer` ou `x-api-key`); sem ela, 401.
    pub fn exigir_chave(&self, chave: &str) -> &Self {
        self.estado().chave = Some(chave.to_owned());
        self
    }

    /// Os pedidos recebidos até agora, na ordem.
    pub fn pedidos(&self) -> Vec<PedidoFalso> {
        self.estado().pedidos.clone()
    }
}

impl Drop for FakeAi {
    fn drop(&mut self) {
        self.parar.store(true, Ordering::SeqCst);
        // Acorda o `accept` para a thread ver o aviso.
        let _ = TcpStream::connect(("127.0.0.1", self.porta));
    }
}

fn ler_pedido(conexao: &mut TcpStream) -> Option<PedidoFalso> {
    let mut dados = Vec::new();
    let mut bloco = [0u8; 4096];
    let fim_cabecalho = loop {
        let n = conexao.read(&mut bloco).ok()?;
        if n == 0 {
            return None;
        }
        dados.extend_from_slice(&bloco[..n]);
        if let Some(pos) = dados.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let cabecalho = String::from_utf8_lossy(&dados[..fim_cabecalho]).into_owned();
    let mut linhas = cabecalho.lines();
    let primeira = linhas.next()?;
    let mut partes = primeira.split_whitespace();
    let metodo = partes.next()?.to_owned();
    let caminho = partes.next()?.to_owned();
    let cabecalhos: Vec<(String, String)> = linhas
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let tamanho = cabecalhos
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    while dados.len() < fim_cabecalho + tamanho {
        let n = conexao.read(&mut bloco).ok()?;
        if n == 0 {
            break;
        }
        dados.extend_from_slice(&bloco[..n]);
    }
    let corpo = String::from_utf8_lossy(&dados[fim_cabecalho..]).into_owned();
    Some(PedidoFalso {
        metodo,
        caminho,
        cabecalhos,
        corpo,
    })
}

fn atender(mut conexao: TcpStream, estado: &Arc<Mutex<Estado>>) {
    let Some(pedido) = ler_pedido(&mut conexao) else {
        return;
    };
    let (roteiro, modelos, por_pagina, chave) = {
        let mut e = estado.lock().expect("estado do FakeAi");
        e.pedidos.push(pedido.clone());
        let roteiro = if e.roteiros.len() > 1 {
            e.roteiros.pop_front()
        } else {
            e.roteiros.front().cloned()
        };
        (roteiro, e.modelos.clone(), e.por_pagina, e.chave.clone())
    };
    let anthropic = pedido.cabecalho("anthropic-version").is_some()
        || pedido.cabecalho("x-api-key").is_some()
        || pedido.caminho.contains("/messages");
    if let Some(chave) = chave {
        let bearer = pedido
            .cabecalho("authorization")
            .and_then(|v| v.strip_prefix("Bearer "));
        let recebida = bearer.or_else(|| pedido.cabecalho("x-api-key"));
        if recebida != Some(chave.as_str()) {
            let corpo = json!({"error": {"message": "Chave inválida"}}).to_string();
            let _ = resposta_simples(&mut conexao, 401, &corpo, None);
            return;
        }
    }
    let (caminho, query) = pedido
        .caminho
        .split_once('?')
        .unwrap_or((pedido.caminho.as_str(), ""));
    let _ = if caminho.ends_with("/models") {
        responder_modelos(&mut conexao, &modelos, anthropic, por_pagina, query)
    } else {
        let roteiro = roteiro.unwrap_or_else(|| Roteiro::novo(Tipo::Partes(vec!["ok".to_owned()])));
        tocar(&mut conexao, &roteiro, anthropic)
    };
    let _ = conexao.flush();
    let _ = conexao.shutdown(Shutdown::Both);
}

fn razao(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        529 => "Overloaded",
        _ => "Status",
    }
}

fn resposta_simples(
    conexao: &mut TcpStream,
    status: u16,
    corpo: &str,
    retry_after: Option<u64>,
) -> std::io::Result<()> {
    let mut cabecalho = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        razao(status),
        corpo.len()
    );
    if let Some(s) = retry_after {
        cabecalho.push_str(&format!("Retry-After: {s}\r\n"));
    }
    cabecalho.push_str("\r\n");
    conexao.write_all(cabecalho.as_bytes())?;
    conexao.write_all(corpo.as_bytes())
}

fn responder_modelos(
    conexao: &mut TcpStream,
    modelos: &[String],
    anthropic: bool,
    por_pagina: Option<usize>,
    query: &str,
) -> std::io::Result<()> {
    let corpo = if anthropic {
        let depois = query
            .split('&')
            .find_map(|p| p.strip_prefix("after_id="))
            .map(str::to_owned);
        let inicio = depois
            .and_then(|id| modelos.iter().position(|m| *m == id))
            .map_or(0, |p| p + 1);
        let tamanho = por_pagina.unwrap_or(usize::MAX);
        let pagina: Vec<&String> = modelos.iter().skip(inicio).take(tamanho).collect();
        let tem_mais = inicio + pagina.len() < modelos.len();
        let dados: Vec<Value> = pagina
            .iter()
            .map(|id| json!({"type": "model", "id": id, "display_name": format!("Modelo {id}")}))
            .collect();
        json!({
            "data": dados,
            "has_more": tem_mais,
            "first_id": pagina.first(),
            "last_id": pagina.last(),
        })
    } else {
        let dados: Vec<Value> = modelos
            .iter()
            .map(|id| json!({"id": id, "object": "model"}))
            .collect();
        json!({"object": "list", "data": dados})
    };
    resposta_simples(conexao, 200, &corpo.to_string(), None)
}

fn escrever(conexao: &mut TcpStream, texto: &str, pausa: Duration) -> std::io::Result<()> {
    conexao.write_all(texto.as_bytes())?;
    conexao.flush()?;
    if !pausa.is_zero() {
        thread::sleep(pausa);
    }
    Ok(())
}

fn tocar(conexao: &mut TcpStream, roteiro: &Roteiro, anthropic: bool) -> std::io::Result<()> {
    match &roteiro.tipo {
        Tipo::Erro { status, corpo } => {
            resposta_simples(conexao, *status, corpo, roteiro.retry_after)
        }
        Tipo::Inteiro(texto) => {
            let corpo = if anthropic {
                json!({
                    "type": "message", "role": "assistant",
                    "content": [{"type": "text", "text": texto}],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 3, "output_tokens": 5},
                })
            } else {
                json!({
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": texto}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 3, "completion_tokens": 5},
                })
            };
            resposta_simples(conexao, 200, &corpo.to_string(), None)
        }
        Tipo::Partes(partes) => {
            conexao.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
            )?;
            if anthropic {
                tocar_anthropic(conexao, roteiro, partes)
            } else {
                tocar_openai(conexao, roteiro, partes)
            }
        }
    }
}

/// As partes que vão antes de fechar a conexão no meio: a metade, arredondada para cima.
fn partes_enviadas<'a>(roteiro: &Roteiro, partes: &'a [String]) -> &'a [String] {
    if roteiro.fechar_no_meio {
        &partes[..partes.len().div_ceil(2)]
    } else {
        partes
    }
}

fn tocar_openai(
    conexao: &mut TcpStream,
    roteiro: &Roteiro,
    partes: &[String],
) -> std::io::Result<()> {
    let pausa = roteiro.pausa;
    if roteiro.extras {
        escrever(conexao, ": keep-alive\n\n", pausa)?;
    }
    if let Some(texto) = &roteiro.raciocinio {
        let evento = json!({"choices": [{"index": 0, "delta": {"reasoning_content": texto}, "finish_reason": null}]});
        escrever(conexao, &format!("data: {evento}\n\n"), pausa)?;
    }
    for parte in partes_enviadas(roteiro, partes) {
        let evento = json!({"id": "fake", "choices": [{"index": 0, "delta": {"content": parte}, "finish_reason": null}]});
        escrever(conexao, &format!("data: {evento}\n\n"), pausa)?;
        if roteiro.extras {
            escrever(conexao, ": keep-alive\n\n", pausa)?;
        }
    }
    if roteiro.fechar_no_meio {
        return Ok(());
    }
    if let Some(mensagem) = &roteiro.erro_no_meio {
        let evento = json!({"error": {"message": mensagem}});
        return escrever(conexao, &format!("data: {evento}\n\n"), pausa);
    }
    let fim = json!({
        "choices": [{"index": 0, "delta": {}, "finish_reason": if roteiro.por_tamanho { "length" } else { "stop" }}],
        "usage": {"prompt_tokens": 3, "completion_tokens": 5},
    });
    escrever(conexao, &format!("data: {fim}\n\n"), pausa)?;
    escrever(conexao, "data: [DONE]\n\n", pausa)
}

fn tocar_anthropic(
    conexao: &mut TcpStream,
    roteiro: &Roteiro,
    partes: &[String],
) -> std::io::Result<()> {
    let pausa = roteiro.pausa;
    let evento = |nome: &str, dados: Value| format!("event: {nome}\ndata: {dados}\n\n");
    escrever(
        conexao,
        &evento(
            "message_start",
            json!({"type": "message_start", "message": {"id": "msg", "usage": {"input_tokens": 12, "output_tokens": 1}}}),
        ),
        pausa,
    )?;
    if roteiro.extras {
        escrever(conexao, &evento("ping", json!({"type": "ping"})), pausa)?;
        escrever(
            conexao,
            &evento("novidade", json!({"type": "tipo_novo", "x": 1})),
            pausa,
        )?;
    }
    escrever(
        conexao,
        &evento(
            "content_block_start",
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        ),
        pausa,
    )?;
    if let Some(texto) = &roteiro.raciocinio {
        let dados = json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": texto}});
        escrever(conexao, &evento("content_block_delta", dados), pausa)?;
    }
    for parte in partes_enviadas(roteiro, partes) {
        let dados = json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": parte}});
        escrever(conexao, &evento("content_block_delta", dados), pausa)?;
        if roteiro.extras {
            escrever(conexao, ": comentário\n\n", pausa)?;
        }
    }
    if roteiro.fechar_no_meio {
        return Ok(());
    }
    if let Some(mensagem) = &roteiro.erro_no_meio {
        let tipo = if mensagem.to_ascii_lowercase().contains("overload") {
            "overloaded_error"
        } else {
            "api_error"
        };
        let dados = json!({"type": "error", "error": {"type": tipo, "message": mensagem}});
        return escrever(conexao, &evento("error", dados), pausa);
    }
    escrever(
        conexao,
        &evento(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
        pausa,
    )?;
    escrever(
        conexao,
        &evento(
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": if roteiro.por_tamanho { "max_tokens" } else { "end_turn" }}, "usage": {"output_tokens": 5}}),
        ),
        pausa,
    )?;
    escrever(
        conexao,
        &evento("message_stop", json!({"type": "message_stop"})),
        pausa,
    )
}
