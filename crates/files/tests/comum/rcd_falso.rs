//! Um `rclone rcd` de teste, em processo: sobe em `127.0.0.1:0`, confere o Basic auth e fala a
//! API de controle remoto do rclone 1.60 sobre árvores de arquivos em memória (um remoto
//! qualquer `nome:` existe sem precisar ser configurado; um caminho que começa por `/` é o
//! disco de verdade). As respostas copiam as chaves do rclone real (maiúsculas e minúsculas).
//!
//! Entende `core/{version,quit,stats}`, `operations/{list,stat,mkdir,rmdir,purge,deletefile,
//! movefile,copyfile,uploadfile,about,fsinfo,publiclink}`, `sync/move`, `job/{status,stop}`,
//! `config/{listremotes,dump,providers,create,update,get,delete}` e o `GET /[remoto:]/caminho`
//! com `Range`. Os provedores falsos são `memoria` (sem perguntas), `falso` (uma opção de cada
//! tipo e uma pergunta em dois passos, `passo1` → `passo2`) e `nuvem` (a pergunta
//! `config_is_local` do OAuth, depois `config_token`).

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};
use tomni_ui_files::rclone::Rcd;

/// O usuário e a senha que o servidor exige.
pub const USUARIO: &str = "usuario-teste";
pub const SENHA: &str = "senha-teste";

/// O que o servidor faz de diferente.
#[derive(Clone)]
pub struct Opcoes {
    /// A versão que `core/version` conta.
    pub versao: [u32; 3],
    /// Quantas consultas ao `job/status` até a cópia terminar.
    pub consultas_da_copia: usize,
    /// A cópia nunca termina sozinha (só com o `job/stop`).
    pub copia_trava: bool,
    /// O remoto não faz links públicos.
    pub sem_link_publico: bool,
    /// O remoto não diz o espaço.
    pub sem_about: bool,
}

impl Default for Opcoes {
    fn default() -> Self {
        Opcoes {
            versao: [1, 60, 1],
            consultas_da_copia: 3,
            copia_trava: false,
            sem_link_publico: false,
            sem_about: false,
        }
    }
}

#[derive(Clone)]
enum No {
    Dir,
    Arquivo(Vec<u8>),
}

#[derive(Clone)]
struct Remoto {
    tipo: String,
    valores: BTreeMap<String, String>,
}

/// O destino de uma cópia que espera o fim da tarefa.
struct Tarefa {
    dados: Option<Vec<u8>>,
    destino: Alvo,
    consultas: usize,
    ate: usize,
    trava: bool,
    parada: bool,
    erro: String,
    terminou: bool,
}

#[derive(Clone)]
enum Alvo {
    /// Um remoto (`nome:`) e a pasta base dentro dele.
    Nuvem(String, String),
    /// O disco de verdade.
    Disco(PathBuf),
}

#[derive(Default)]
struct Estado {
    arvores: HashMap<String, BTreeMap<String, No>>,
    remotos: BTreeMap<String, Remoto>,
    tarefas: HashMap<i64, Tarefa>,
    proxima_tarefa: i64,
    pedidos: Vec<String>,
}

struct Compartilhado {
    estado: Mutex<Estado>,
    opcoes: Opcoes,
}

/// O servidor. Para ao ser solto.
pub struct RcdFalso {
    endereco: SocketAddr,
    compartilhado: Arc<Compartilhado>,
    parar: Arc<AtomicBool>,
}

impl Drop for RcdFalso {
    fn drop(&mut self) {
        self.parar.store(true, Ordering::SeqCst);
        // Acorda o `accept`.
        let _ = TcpStream::connect(self.endereco);
    }
}

impl RcdFalso {
    pub fn iniciar() -> RcdFalso {
        RcdFalso::com(Opcoes::default())
    }

    pub fn com(opcoes: Opcoes) -> RcdFalso {
        let escuta = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let endereco = escuta.local_addr().unwrap();
        let compartilhado = Arc::new(Compartilhado {
            estado: Mutex::new(Estado::default()),
            opcoes,
        });
        let parar = Arc::new(AtomicBool::new(false));
        {
            let (compartilhado, parar) = (Arc::clone(&compartilhado), Arc::clone(&parar));
            thread::spawn(move || {
                for conexao in escuta.incoming() {
                    if parar.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(conexao) = conexao else { continue };
                    let compartilhado = Arc::clone(&compartilhado);
                    thread::spawn(move || atender(conexao, &compartilhado));
                }
            });
        }
        RcdFalso {
            endereco,
            compartilhado,
            parar,
        }
    }

    /// O endereço (`http://127.0.0.1:PORTA`).
    pub fn url(&self) -> String {
        format!("http://{}", self.endereco)
    }

    /// Um `Rcd` ligado a este servidor, com o usuário e a senha certos.
    pub fn rcd(&self) -> Arc<Rcd> {
        Rcd::attach(&self.url(), USUARIO, SENHA)
    }

    /// Um `Rcd` com a senha errada.
    pub fn rcd_com_senha_errada(&self) -> Arc<Rcd> {
        Rcd::attach(&self.url(), USUARIO, "outra-senha")
    }

    fn estado(&self) -> std::sync::MutexGuard<'_, Estado> {
        self.compartilhado
            .estado
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Põe um arquivo em `remoto` (`m:`), no caminho `caminho` (`dir/a.txt`).
    pub fn semear(&self, remoto: &str, caminho: &str, dados: &[u8]) {
        let mut estado = self.estado();
        escrever_na_arvore(&mut estado, remoto, caminho, Some(dados.to_vec()));
    }

    /// Os bytes de um arquivo do remoto.
    pub fn arquivo(&self, remoto: &str, caminho: &str) -> Option<Vec<u8>> {
        let estado = self.estado();
        match estado.arvores.get(remoto)?.get(caminho.trim_matches('/'))? {
            No::Arquivo(dados) => Some(dados.clone()),
            No::Dir => None,
        }
    }

    /// Os caminhos (arquivos e pastas) de um remoto.
    pub fn caminhos(&self, remoto: &str) -> Vec<String> {
        let estado = self.estado();
        estado
            .arvores
            .get(remoto)
            .map(|a| a.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Cada pedido recebido: `método corpo` (o corpo do JSON, sem os arquivos).
    pub fn pedidos(&self) -> Vec<String> {
        self.estado().pedidos.clone()
    }

    /// Os pedidos que começam por `metodo`.
    pub fn pedidos_de(&self, metodo: &str) -> Vec<String> {
        self.pedidos()
            .into_iter()
            .filter(|p| p.split(' ').next() == Some(metodo))
            .collect()
    }

    /// Os valores guardados de um remoto da configuração.
    pub fn valores_do_remoto(&self, nome: &str) -> Option<BTreeMap<String, String>> {
        self.estado().remotos.get(nome).map(|r| r.valores.clone())
    }

    /// Cria um remoto da configuração sem passar pela API.
    pub fn configurar(&self, nome: &str, tipo: &str) {
        self.estado().remotos.insert(
            nome.to_owned(),
            Remoto {
                tipo: tipo.to_owned(),
                valores: BTreeMap::new(),
            },
        );
    }
}

fn escrever_na_arvore(estado: &mut Estado, remoto: &str, caminho: &str, dados: Option<Vec<u8>>) {
    let arvore = estado.arvores.entry(remoto.to_owned()).or_default();
    let caminho = caminho.trim_matches('/').to_owned();
    let mut acumulado = String::new();
    let partes: Vec<&str> = caminho.split('/').collect();
    for pai in &partes[..partes.len() - 1] {
        if !acumulado.is_empty() {
            acumulado.push('/');
        }
        acumulado.push_str(pai);
        arvore.entry(acumulado.clone()).or_insert(No::Dir);
    }
    arvore.insert(
        caminho,
        match dados {
            Some(dados) => No::Arquivo(dados),
            None => No::Dir,
        },
    );
}

// ---------------------------------------------------------------- HTTP

struct Pedido {
    metodo: String,
    caminho: String,
    consulta: Vec<(String, String)>,
    cabecalhos: HashMap<String, String>,
    corpo: Vec<u8>,
}

impl Pedido {
    fn q(&self, nome: &str) -> Option<&str> {
        self.consulta
            .iter()
            .find(|(k, _)| k == nome)
            .map(|(_, v)| v.as_str())
    }

    fn h(&self, nome: &str) -> Option<&str> {
        self.cabecalhos.get(nome).map(String::as_str)
    }
}

struct Resposta {
    status: u16,
    cabecalhos: Vec<(String, String)>,
    corpo: Vec<u8>,
}

fn json_ok(valor: Value) -> Resposta {
    Resposta {
        status: 200,
        cabecalhos: vec![("Content-Type".into(), "application/json".into())],
        corpo: serde_json::to_vec_pretty(&valor).unwrap(),
    }
}

fn erro(status: u16, mensagem: &str, caminho: &str) -> Resposta {
    Resposta {
        status,
        cabecalhos: vec![("Content-Type".into(), "application/json".into())],
        corpo: serde_json::to_vec(&json!({
            "error": mensagem,
            "input": {},
            "path": caminho,
            "status": status,
        }))
        .unwrap(),
    }
}

fn ler_pedido(leitor: &mut BufReader<TcpStream>) -> Option<Pedido> {
    let mut linha = String::new();
    if leitor.read_line(&mut linha).ok()? == 0 {
        return None;
    }
    let mut partes = linha.split_whitespace();
    let metodo = partes.next()?.to_owned();
    let alvo = partes.next()?.to_owned();
    let (caminho, consulta) = match alvo.split_once('?') {
        Some((c, q)) => (c.to_owned(), q.to_owned()),
        None => (alvo, String::new()),
    };
    let consulta = consulta
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|par| {
            let (k, v) = par.split_once('=').unwrap_or((par, ""));
            (
                percent_decode_str(k).decode_utf8_lossy().into_owned(),
                percent_decode_str(v).decode_utf8_lossy().into_owned(),
            )
        })
        .collect();
    let mut cabecalhos = HashMap::new();
    loop {
        let mut linha = String::new();
        if leitor.read_line(&mut linha).ok()? == 0 {
            return None;
        }
        let linha = linha.trim_end();
        if linha.is_empty() {
            break;
        }
        if let Some((k, v)) = linha.split_once(':') {
            cabecalhos.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let mut corpo = Vec::new();
    if cabecalhos
        .get("transfer-encoding")
        .is_some_and(|v| v.eq_ignore_ascii_case("chunked"))
    {
        loop {
            let mut tamanho = String::new();
            leitor.read_line(&mut tamanho).ok()?;
            let tamanho = usize::from_str_radix(tamanho.trim().split(';').next()?, 16).ok()?;
            if tamanho == 0 {
                let mut fim = String::new();
                leitor.read_line(&mut fim).ok()?;
                break;
            }
            let mut pedaco = vec![0u8; tamanho];
            leitor.read_exact(&mut pedaco).ok()?;
            corpo.extend_from_slice(&pedaco);
            let mut crlf = [0u8; 2];
            leitor.read_exact(&mut crlf).ok()?;
        }
    } else if let Some(n) = cabecalhos
        .get("content-length")
        .and_then(|v| v.parse().ok())
    {
        corpo = vec![0u8; n];
        leitor.read_exact(&mut corpo).ok()?;
    }
    Some(Pedido {
        metodo,
        caminho,
        consulta,
        cabecalhos,
        corpo,
    })
}

fn atender(conexao: TcpStream, compartilhado: &Compartilhado) {
    let _ = conexao.set_read_timeout(Some(Duration::from_secs(30)));
    let Ok(escrita) = conexao.try_clone() else {
        return;
    };
    let mut escrita = escrita;
    let mut leitor = BufReader::new(conexao);
    while let Some(pedido) = ler_pedido(&mut leitor) {
        let resposta = responder(&pedido, compartilhado);
        let mut cabeca = format!(
            "HTTP/1.1 {} X\r\nContent-Length: {}\r\n",
            resposta.status,
            resposta.corpo.len()
        );
        for (k, v) in &resposta.cabecalhos {
            cabeca.push_str(&format!("{k}: {v}\r\n"));
        }
        cabeca.push_str("\r\n");
        if escrita.write_all(cabeca.as_bytes()).is_err()
            || (pedido.metodo != "HEAD" && escrita.write_all(&resposta.corpo).is_err())
        {
            return;
        }
    }
}

fn autorizado(pedido: &Pedido) -> bool {
    let esperado = format!("Basic {}", STANDARD.encode(format!("{USUARIO}:{SENHA}")));
    pedido.h("authorization") == Some(esperado.as_str())
}

fn responder(pedido: &Pedido, compartilhado: &Compartilhado) -> Resposta {
    if !autorizado(pedido) {
        return Resposta {
            status: 401,
            cabecalhos: vec![
                ("Content-Type".into(), "text/plain".into()),
                ("WWW-Authenticate".into(), "Basic realm=\"rclone\"".into()),
            ],
            corpo: b"Unauthorized\n".to_vec(),
        };
    }
    let mut estado = compartilhado
        .estado
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if pedido.metodo == "GET" {
        return servir(pedido, &estado);
    }
    let metodo = pedido.caminho.trim_start_matches('/').to_owned();
    let corpo_json: Value = if pedido
        .h("content-type")
        .is_some_and(|t| t.starts_with("application/json"))
        && !pedido.corpo.is_empty()
    {
        serde_json::from_slice(&pedido.corpo).unwrap_or(Value::Null)
    } else {
        json!({})
    };
    let registro = if metodo == "operations/uploadfile" {
        format!(
            "{metodo} fs={} remote={}",
            pedido.q("fs").unwrap_or(""),
            pedido.q("remote").unwrap_or("")
        )
    } else {
        format!("{metodo} {corpo_json}")
    };
    estado.pedidos.push(registro);
    match despachar(
        &metodo,
        &corpo_json,
        pedido,
        &mut estado,
        &compartilhado.opcoes,
    ) {
        Ok(valor) => json_ok(valor),
        Err(resposta) => resposta,
    }
}

// ---------------------------------------------------------------- API

type R = Result<Value, Resposta>;

fn texto<'a>(corpo: &'a Value, chave: &str, metodo: &str) -> Result<&'a str, Resposta> {
    corpo.get(chave).and_then(Value::as_str).ok_or_else(|| {
        erro(
            400,
            &format!("Didn't find key \"{chave}\" in input"),
            metodo,
        )
    })
}

/// Separa `m:sub/pasta` em `("m:", "sub/pasta")`; um caminho absoluto é o disco.
fn alvo_de(spec: &str) -> Alvo {
    if spec.starts_with('/') {
        return Alvo::Disco(PathBuf::from(spec));
    }
    match spec.split_once(':') {
        Some((nome, base)) => Alvo::Nuvem(format!("{nome}:"), base.trim_matches('/').to_owned()),
        None => Alvo::Disco(PathBuf::from(spec)),
    }
}

fn juntar(base: &str, rel: &str) -> String {
    let rel = rel.trim_matches('/');
    match (base.is_empty(), rel.is_empty()) {
        (true, _) => rel.to_owned(),
        (_, true) => base.to_owned(),
        _ => format!("{base}/{rel}"),
    }
}

fn existe_pasta(arvore: &BTreeMap<String, No>, caminho: &str) -> bool {
    if caminho.is_empty() {
        return true;
    }
    let prefixo = format!("{caminho}/");
    matches!(arvore.get(caminho), Some(No::Dir)) || arvore.keys().any(|k| k.starts_with(&prefixo))
}

fn data_rfc3339() -> &'static str {
    "2026-10-18T12:30:45.123456789Z"
}

fn item(nome: &str, caminho: &str, no: &No) -> Value {
    match no {
        No::Dir => json!({
            "Path": caminho, "Name": nome, "Size": -1, "MimeType": "inode/directory",
            "ModTime": data_rfc3339(), "IsDir": true, "ID": "",
        }),
        No::Arquivo(dados) => json!({
            "Path": caminho, "Name": nome, "Size": dados.len(), "MimeType": "application/octet-stream",
            "ModTime": data_rfc3339(), "IsDir": false, "ID": "",
        }),
    }
}

fn nome_de(caminho: &str) -> &str {
    caminho.rsplit('/').next().unwrap_or(caminho)
}

fn ler_alvo(estado: &Estado, alvo: &Alvo, rel: &str) -> Result<Vec<u8>, Resposta> {
    match alvo {
        Alvo::Nuvem(remoto, base) => {
            match estado
                .arvores
                .get(remoto)
                .and_then(|a| a.get(&juntar(base, rel)))
            {
                Some(No::Arquivo(dados)) => Ok(dados.clone()),
                _ => Err(erro(404, "object not found", "operations/copyfile")),
            }
        }
        Alvo::Disco(pasta) => std::fs::read(pasta.join(rel))
            .map_err(|_| erro(404, "object not found", "operations/copyfile")),
    }
}

fn gravar_alvo(
    estado: &mut Estado,
    alvo: &Alvo,
    rel: &str,
    dados: Vec<u8>,
) -> Result<(), Resposta> {
    match alvo {
        Alvo::Nuvem(remoto, base) => {
            escrever_na_arvore(estado, remoto, &juntar(base, rel), Some(dados));
            Ok(())
        }
        Alvo::Disco(pasta) => std::fs::write(pasta.join(rel), dados).map_err(|e| {
            erro(
                500,
                &format!("falhou ao gravar: {e}"),
                "operations/copyfile",
            )
        }),
    }
}

fn despachar(
    metodo: &str,
    corpo: &Value,
    pedido: &Pedido,
    estado: &mut Estado,
    opcoes: &Opcoes,
) -> R {
    match metodo {
        "core/version" => Ok(json!({
            "version": format!("v{}.{}.{}", opcoes.versao[0], opcoes.versao[1], opcoes.versao[2]),
            "decomposed": opcoes.versao,
            "isGit": false, "os": "linux", "arch": "amd64", "goVersion": "go1.21",
        })),
        "core/quit" => Ok(json!({})),
        "operations/list" => {
            let Alvo::Nuvem(remoto, base) = alvo_de(texto(corpo, "fs", metodo)?) else {
                return Err(erro(400, "só remotos", metodo));
            };
            let rel = juntar(&base, texto(corpo, "remote", metodo)?);
            let arvore = estado.arvores.entry(remoto).or_default();
            if !existe_pasta(arvore, &rel) {
                return Err(erro(404, "directory not found", metodo));
            }
            let prefixo = if rel.is_empty() {
                String::new()
            } else {
                format!("{rel}/")
            };
            let itens: Vec<Value> = arvore
                .iter()
                .filter(|(k, _)| {
                    k.strip_prefix(&prefixo)
                        .is_some_and(|resto| !resto.is_empty() && !resto.contains('/'))
                })
                .map(|(k, no)| {
                    item(
                        nome_de(k),
                        k.strip_prefix(&format!("{base}/")).unwrap_or(k),
                        no,
                    )
                })
                .collect();
            Ok(json!({ "list": itens }))
        }
        "operations/stat" => {
            let Alvo::Nuvem(remoto, base) = alvo_de(texto(corpo, "fs", metodo)?) else {
                return Err(erro(400, "só remotos", metodo));
            };
            let rel = juntar(&base, texto(corpo, "remote", metodo)?);
            let arvore = estado.arvores.entry(remoto).or_default();
            Ok(match arvore.get(&rel) {
                Some(no) => json!({ "item": item(nome_de(&rel), &rel, no) }),
                None if existe_pasta(arvore, &rel) => {
                    json!({ "item": item(nome_de(&rel), &rel, &No::Dir) })
                }
                None => json!({ "item": null }),
            })
        }
        "operations/mkdir" => {
            let Alvo::Nuvem(remoto, base) = alvo_de(texto(corpo, "fs", metodo)?) else {
                return Err(erro(400, "só remotos", metodo));
            };
            let rel = juntar(&base, texto(corpo, "remote", metodo)?);
            if !rel.is_empty() {
                escrever_na_arvore(estado, &remoto, &rel, None);
            }
            Ok(json!({}))
        }
        "operations/rmdir" | "operations/purge" => {
            let Alvo::Nuvem(remoto, base) = alvo_de(texto(corpo, "fs", metodo)?) else {
                return Err(erro(400, "só remotos", metodo));
            };
            let rel = juntar(&base, texto(corpo, "remote", metodo)?);
            let arvore = estado.arvores.entry(remoto).or_default();
            if !existe_pasta(arvore, &rel) {
                return Err(erro(404, "directory not found", metodo));
            }
            let prefixo = format!("{rel}/");
            if metodo == "operations/rmdir" && arvore.keys().any(|k| k.starts_with(&prefixo)) {
                return Err(erro(500, "directory not empty", metodo));
            }
            arvore.retain(|k, _| *k != rel && !k.starts_with(&prefixo));
            Ok(json!({}))
        }
        "operations/deletefile" => {
            let Alvo::Nuvem(remoto, base) = alvo_de(texto(corpo, "fs", metodo)?) else {
                return Err(erro(400, "só remotos", metodo));
            };
            let rel = juntar(&base, texto(corpo, "remote", metodo)?);
            match estado.arvores.entry(remoto).or_default().remove(&rel) {
                Some(_) => Ok(json!({})),
                None => Err(erro(404, "object not found", metodo)),
            }
        }
        "operations/movefile" => {
            let origem = alvo_de(texto(corpo, "srcFs", metodo)?);
            let destino = alvo_de(texto(corpo, "dstFs", metodo)?);
            let dados = ler_alvo(estado, &origem, texto(corpo, "srcRemote", metodo)?)?;
            gravar_alvo(estado, &destino, texto(corpo, "dstRemote", metodo)?, dados)?;
            if let Alvo::Nuvem(remoto, base) = origem {
                let rel = juntar(&base, texto(corpo, "srcRemote", metodo)?);
                estado.arvores.entry(remoto).or_default().remove(&rel);
            }
            Ok(json!({}))
        }
        "sync/move" => {
            let (Alvo::Nuvem(r_origem, b_origem), Alvo::Nuvem(r_destino, b_destino)) = (
                alvo_de(texto(corpo, "srcFs", metodo)?),
                alvo_de(texto(corpo, "dstFs", metodo)?),
            ) else {
                return Err(erro(400, "só remotos", metodo));
            };
            let prefixo = format!("{b_origem}/");
            let movidos: Vec<(String, No)> = estado
                .arvores
                .entry(r_origem.clone())
                .or_default()
                .iter()
                .filter_map(|(k, no)| k.strip_prefix(&prefixo).map(|r| (r.to_owned(), no.clone())))
                .collect();
            if movidos.is_empty() {
                return Err(erro(404, "directory not found", metodo));
            }
            for (rel, no) in movidos {
                let dados = match no {
                    No::Arquivo(dados) => Some(dados),
                    No::Dir => None,
                };
                escrever_na_arvore(estado, &r_destino, &juntar(&b_destino, &rel), dados);
                estado
                    .arvores
                    .entry(r_origem.clone())
                    .or_default()
                    .remove(&format!("{prefixo}{rel}"));
            }
            Ok(json!({}))
        }
        "operations/copyfile" => {
            let origem = alvo_de(texto(corpo, "srcFs", metodo)?);
            let destino = alvo_de(texto(corpo, "dstFs", metodo)?);
            let src = texto(corpo, "srcRemote", metodo)?;
            let dst = texto(corpo, "dstRemote", metodo)?.to_owned();
            if corpo.get("_async").and_then(Value::as_bool) == Some(true) {
                estado.proxima_tarefa += 1;
                let id = estado.proxima_tarefa;
                let (dados, erro_inicial) = match ler_alvo(estado, &origem, src) {
                    Ok(d) => (Some(d), String::new()),
                    Err(_) => (None, "object not found".to_owned()),
                };
                let terminou = dados.is_none();
                estado.tarefas.insert(
                    id,
                    Tarefa {
                        dados,
                        destino: match destino {
                            Alvo::Nuvem(r, b) => Alvo::Nuvem(r, juntar(&b, &dst)),
                            Alvo::Disco(p) => Alvo::Disco(p.join(&dst)),
                        },
                        consultas: 0,
                        ate: opcoes.consultas_da_copia,
                        trava: opcoes.copia_trava,
                        parada: false,
                        erro: erro_inicial,
                        terminou,
                    },
                );
                Ok(json!({ "jobid": id, "executeId": "x" }))
            } else {
                let dados = ler_alvo(estado, &origem, src)?;
                gravar_alvo(estado, &destino, &dst, dados)?;
                Ok(json!({}))
            }
        }
        "operations/uploadfile" => {
            let Alvo::Nuvem(remoto, base) = alvo_de(pedido.q("fs").unwrap_or("")) else {
                return Err(erro(400, "só remotos", metodo));
            };
            let pasta = juntar(&base, pedido.q("remote").unwrap_or(""));
            let (nome, dados) = ler_multipart(pedido)
                .ok_or_else(|| erro(400, "request didn't contain a file", metodo))?;
            escrever_na_arvore(estado, &remoto, &juntar(&pasta, &nome), Some(dados));
            Ok(json!({}))
        }
        "operations/fsinfo" => Ok(json!({
            "Name": "falso", "Root": "", "String": "falso",
            "Features": {
                "Copy": true, "Move": true, "DirMove": true,
                "PublicLink": !opcoes.sem_link_publico,
                "About": !opcoes.sem_about,
                "CanHaveEmptyDirectories": true,
            },
        })),
        "operations/about" => {
            if opcoes.sem_about {
                return Err(erro(500, "about not supported", metodo));
            }
            let Alvo::Nuvem(remoto, _) = alvo_de(texto(corpo, "fs", metodo)?) else {
                return Err(erro(400, "só remotos", metodo));
            };
            let usado: u64 = estado
                .arvores
                .get(&remoto)
                .map(|a| {
                    a.values()
                        .map(|n| match n {
                            No::Arquivo(d) => d.len() as u64,
                            No::Dir => 0,
                        })
                        .sum()
                })
                .unwrap_or(0);
            Ok(json!({ "total": 10_000_000u64, "used": usado, "free": 10_000_000u64 - usado }))
        }
        "operations/publiclink" => {
            if opcoes.sem_link_publico {
                return Err(erro(500, "optional feature not implemented", metodo));
            }
            Ok(
                json!({ "url": format!("https://exemplo.test/{}", texto(corpo, "remote", metodo)?) }),
            )
        }
        "job/status" => {
            let id = corpo.get("jobid").and_then(Value::as_i64).unwrap_or(0);
            let Some(tarefa) = estado.tarefas.get_mut(&id) else {
                return Err(erro(404, "job not found", metodo));
            };
            tarefa.consultas += 1;
            let terminar = !tarefa.terminou && !tarefa.trava && tarefa.consultas >= tarefa.ate;
            let (finished, error) = (tarefa.terminou || terminar, tarefa.erro.clone());
            if terminar {
                let destino = tarefa.destino.clone();
                let dados = tarefa.dados.clone().unwrap_or_default();
                tarefa.terminou = true;
                match destino {
                    Alvo::Nuvem(r, caminho) => {
                        escrever_na_arvore(estado, &r, &caminho, Some(dados))
                    }
                    Alvo::Disco(p) => {
                        let _ = std::fs::write(p, dados);
                    }
                }
            }
            Ok(json!({
                "finished": finished, "success": finished && error.is_empty(), "error": error,
                "id": id, "duration": 0.1, "startTime": data_rfc3339(), "endTime": data_rfc3339(),
                "output": {},
            }))
        }
        "job/stop" => {
            let id = corpo.get("jobid").and_then(Value::as_i64).unwrap_or(0);
            if let Some(tarefa) = estado.tarefas.get_mut(&id) {
                tarefa.terminou = true;
                tarefa.parada = true;
                tarefa.erro = "context canceled".to_owned();
            }
            Ok(json!({}))
        }
        "core/stats" => {
            let grupo = corpo.get("group").and_then(Value::as_str).unwrap_or("");
            let id: i64 = grupo
                .strip_prefix("job/")
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            let (feito, total) = estado
                .tarefas
                .get(&id)
                .map(|t| {
                    let total = t.dados.as_ref().map_or(0, |d| d.len() as u64);
                    let passo = t.consultas.min(t.ate.max(1)) as u64;
                    (total * passo / t.ate.max(1) as u64, total)
                })
                .unwrap_or((0, 0));
            Ok(json!({ "bytes": feito, "totalBytes": total, "transfers": 0, "errors": 0 }))
        }
        m if m.starts_with("config/") => config(m, corpo, estado),
        outro => Err(erro(
            404,
            &format!("couldn't find method \"{outro}\""),
            outro,
        )),
    }
}

/// O arquivo do `multipart/form-data`: o nome e o conteúdo.
fn ler_multipart(pedido: &Pedido) -> Option<(String, Vec<u8>)> {
    let tipo = pedido.h("content-type")?;
    let fronteira = tipo.split("boundary=").nth(1)?.trim_matches('"').to_owned();
    let marca = format!("--{fronteira}").into_bytes();
    let corpo = &pedido.corpo;
    let inicio = achar(corpo, &marca, 0)? + marca.len();
    let fim_da_cabeca = achar(corpo, b"\r\n\r\n", inicio)?;
    let cabeca = String::from_utf8_lossy(&corpo[inicio..fim_da_cabeca]).into_owned();
    let nome = cabeca
        .split("filename=\"")
        .nth(1)?
        .split('"')
        .next()?
        .to_owned();
    let dados_inicio = fim_da_cabeca + 4;
    let fecho = achar(corpo, &[b"\r\n".as_slice(), &marca].concat(), dados_inicio)?;
    Some((nome, corpo[dados_inicio..fecho].to_vec()))
}

fn achar(palheiro: &[u8], agulha: &[u8], a_partir_de: usize) -> Option<usize> {
    palheiro
        .get(a_partir_de..)?
        .windows(agulha.len())
        .position(|janela| janela == agulha)
        .map(|p| p + a_partir_de)
}

/// O `GET /[remoto:]/caminho` do `--rc-serve`, com `Range`.
fn servir(pedido: &Pedido, estado: &Estado) -> Resposta {
    let caminho = percent_decode_str(&pedido.caminho)
        .decode_utf8_lossy()
        .into_owned();
    let caminho = caminho.trim_start_matches('/');
    let Some(resto) = caminho.strip_prefix('[') else {
        return erro(404, "not found", "serve");
    };
    let Some((spec, rel)) = resto.split_once("]/") else {
        return erro(404, "not found", "serve");
    };
    let Alvo::Nuvem(remoto, base) = alvo_de(spec) else {
        return erro(404, "not found", "serve");
    };
    let Some(No::Arquivo(dados)) = estado
        .arvores
        .get(&remoto)
        .and_then(|a| a.get(&juntar(&base, rel)))
    else {
        return Resposta {
            status: 404,
            cabecalhos: vec![("Content-Type".into(), "text/plain".into())],
            corpo: b"404 page not found\n".to_vec(),
        };
    };
    let total = dados.len();
    let faixa = pedido.h("range").and_then(|r| r.strip_prefix("bytes="));
    let Some(faixa) = faixa else {
        return Resposta {
            status: 200,
            cabecalhos: vec![("Content-Type".into(), "application/octet-stream".into())],
            corpo: dados.clone(),
        };
    };
    let (de, ate) = faixa.split_once('-').unwrap_or((faixa, ""));
    let de: usize = de.parse().unwrap_or(0);
    let ate: usize = ate
        .parse()
        .unwrap_or(total.saturating_sub(1))
        .min(total.saturating_sub(1));
    if de >= total {
        return Resposta {
            status: 416,
            cabecalhos: vec![("Content-Range".into(), format!("bytes */{total}"))],
            corpo: Vec::new(),
        };
    }
    Resposta {
        status: 206,
        cabecalhos: vec![
            ("Content-Type".into(), "application/octet-stream".into()),
            ("Content-Range".into(), format!("bytes {de}-{ate}/{total}")),
        ],
        corpo: dados[de..=ate].to_vec(),
    }
}

// ---------------------------------------------------------------- config

fn opcao(nome: &str, ajuda: &str, tipo: &str, extra: Value) -> Value {
    let mut o = json!({
        "Name": nome, "Help": ajuda, "Provider": "", "Default": "", "Value": null,
        "ShortOpt": "", "Hide": 0, "Required": false, "IsPassword": false, "NoPrefix": false,
        "Advanced": false, "Exclusive": false, "DefaultStr": "", "ValueStr": "", "Type": tipo,
    });
    for (k, v) in extra.as_object().unwrap() {
        o[k] = v.clone();
    }
    o
}

fn provedores() -> Value {
    json!([
        {
            "Name": "memoria", "Description": "Memória falsa", "Prefix": "memoria",
            "Options": [],
        },
        {
            "Name": "nuvem", "Description": "Nuvem falsa com login", "Prefix": "nuvem",
            "Options": [
                opcao("client_id", "ID do aplicativo.\nOpcional.", "string", json!({})),
            ],
        },
        {
            "Name": "falso", "Description": "Provedor falso", "Prefix": "falso",
            "Options": [
                opcao("provider", "Qual provedor.", "string", json!({
                    "Exclusive": true,
                    "Examples": [
                        {"Value": "AWS", "Help": "Amazon", "Provider": ""},
                        {"Value": "Outro", "Help": "Qualquer outro", "Provider": ""},
                    ],
                })),
                opcao("url", "O endereço do servidor.\nEx.: https://exemplo.test", "string", json!({"Required": true})),
                opcao("usar_tls", "Usar TLS.", "bool", json!({"Default": false, "DefaultStr": "false", "ValueStr": "false"})),
                opcao("modo", "O modo de operação.", "string", json!({
                    "Exclusive": true, "Default": "a", "DefaultStr": "a",
                    "Examples": [
                        {"Value": "a", "Help": "Modo A", "Provider": ""},
                        {"Value": "b", "Help": "Modo B", "Provider": ""},
                    ],
                })),
                opcao("exemplo", "Um texto com sugestões.", "string", json!({
                    "Examples": [{"Value": "um", "Help": "Primeiro", "Provider": ""}],
                })),
                opcao("senha", "A senha.", "string", json!({"IsPassword": true})),
                opcao("avancada", "Opção avançada.", "int", json!({"Advanced": true, "Default": 4, "DefaultStr": "4"})),
                opcao("escondida", "Não aparece no formulário.", "string", json!({"Hide": 2})),
                opcao("so_aws", "Só para a AWS.", "string", json!({"Provider": "AWS"})),
                opcao("menos_aws", "Para todos menos a AWS.", "string", json!({"Provider": "!AWS"})),
            ],
        },
    ])
}

fn passo(estado_nome: &str, nome: &str, ajuda: &str, tipo: &str, exemplos: Value) -> Value {
    json!({
        "State": estado_nome,
        "Option": opcao(nome, ajuda, tipo, json!({"Examples": exemplos, "Exclusive": tipo == "bool"})),
        "Error": "",
        "Result": "",
    })
}

fn texto_do_parametro(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        outro => outro.to_string(),
    }
}

fn guardar(estado: &mut Estado, nome: &str, tipo: Option<&str>, parametros: &Value, ofuscar: bool) {
    let senhas: Vec<String> = provedores()
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| tipo.is_none_or(|t| p["Name"] == t))
        .flat_map(|p| p["Options"].as_array().unwrap().clone())
        .filter(|o| o["IsPassword"] == true)
        .map(|o| o["Name"].as_str().unwrap().to_owned())
        .collect();
    let remoto = estado.remotos.entry(nome.to_owned()).or_insert(Remoto {
        tipo: tipo.unwrap_or("").to_owned(),
        valores: BTreeMap::new(),
    });
    if let Some(tipo) = tipo {
        remoto.tipo = tipo.to_owned();
    }
    if let Some(mapa) = parametros.as_object() {
        for (k, v) in mapa {
            let mut valor = texto_do_parametro(v);
            if ofuscar && senhas.contains(k) {
                valor = format!("ofuscado:{}", valor.chars().rev().collect::<String>());
            }
            remoto.valores.insert(k.clone(), valor);
        }
    }
}

fn config(metodo: &str, corpo: &Value, estado: &mut Estado) -> R {
    let nome = corpo.get("name").and_then(Value::as_str).unwrap_or("");
    let opt = corpo.get("opt").cloned().unwrap_or(json!({}));
    let ofuscar = opt.get("obscure").and_then(Value::as_bool) == Some(true);
    match metodo {
        "config/listremotes" => Ok(json!({ "remotes": estado.remotos.keys().collect::<Vec<_>>() })),
        "config/dump" => {
            let mut mapa = Map::new();
            for (nome, r) in &estado.remotos {
                let mut valores = Map::new();
                valores.insert("type".into(), json!(r.tipo));
                for (k, v) in &r.valores {
                    valores.insert(k.clone(), json!(v));
                }
                mapa.insert(nome.clone(), Value::Object(valores));
            }
            Ok(Value::Object(mapa))
        }
        "config/providers" => Ok(json!({ "providers": provedores() })),
        "config/get" => {
            let r = estado
                .remotos
                .get(nome)
                .ok_or_else(|| erro(404, "didn't find section in config file", metodo))?;
            let mut mapa = Map::new();
            mapa.insert("type".into(), json!(r.tipo));
            for (k, v) in &r.valores {
                mapa.insert(k.clone(), json!(v));
            }
            Ok(Value::Object(mapa))
        }
        "config/delete" => {
            estado.remotos.remove(nome);
            Ok(json!({}))
        }
        "config/create" => {
            let tipo = corpo.get("type").and_then(Value::as_str).unwrap_or("");
            if estado.remotos.contains_key(nome) {
                return Err(erro(
                    500,
                    &format!("config name already exists: {nome}"),
                    metodo,
                ));
            }
            guardar(estado, nome, Some(tipo), &corpo["parameters"], ofuscar);
            Ok(primeira_pergunta(tipo))
        }
        "config/update" => {
            if !estado.remotos.contains_key(nome) {
                return Err(erro(404, "didn't find section in config file", metodo));
            }
            guardar(estado, nome, None, &corpo["parameters"], ofuscar);
            if opt.get("continue").and_then(Value::as_bool) == Some(true) {
                let tipo = estado.remotos[nome].tipo.clone();
                let estado_atual = opt.get("state").and_then(Value::as_str).unwrap_or("");
                let resposta = opt.get("result").and_then(Value::as_str).unwrap_or("");
                return Ok(proxima_pergunta(&tipo, estado_atual, resposta));
            }
            Ok(json!({}))
        }
        outro => Err(erro(
            404,
            &format!("couldn't find method \"{outro}\""),
            outro,
        )),
    }
}

fn primeira_pergunta(tipo: &str) -> Value {
    match tipo {
        "falso" => passo(
            "passo1",
            "pergunta1",
            "Primeira pergunta do remoto falso.",
            "string",
            json!([]),
        ),
        "nuvem" => passo(
            "oauth",
            "config_is_local",
            "Use auto config?\n * Say Y if not sure\n * Say N if you are working on a remote or headless machine",
            "bool",
            json!([{"Value": "true", "Help": "Yes", "Provider": ""}, {"Value": "false", "Help": "No", "Provider": ""}]),
        ),
        _ => json!({}),
    }
}

fn proxima_pergunta(tipo: &str, estado: &str, resposta: &str) -> Value {
    match (tipo, estado) {
        ("falso", "passo1") => passo(
            "passo2",
            "pergunta2",
            "Segunda pergunta do remoto falso.",
            "string",
            json!([]),
        ),
        ("nuvem", "oauth") if resposta == "false" => passo(
            "token",
            "config_token",
            "Paste the result of `rclone authorize`.",
            "string",
            json!([]),
        ),
        ("nuvem", "oauth") => {
            // O login no navegador: a chamada só volta quando termina.
            thread::sleep(Duration::from_millis(300));
            json!({})
        }
        _ => json!({}),
    }
}
