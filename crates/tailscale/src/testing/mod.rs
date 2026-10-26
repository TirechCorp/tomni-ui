//! O plano de controle falso, para os testes do crate e dos que dependem dele.
//!
//! O [`ControlFake`] fala o protocolo de verdade em HTTP puro na porta local: `GET /key`, o
//! upgrade `/ts2021` com o handshake Noise (o lado do servidor), o HTTP/2 por cima, o
//! `POST /machine/register` e o `POST /machine/map` (o mapa inteiro e, depois, os deltas que o
//! teste manda, com `zstd` ou sem). Guarda cada pedido que recebe, para os testes conferirem o
//! que o cliente declarou (por exemplo, `ShieldsUp` ligado e nenhuma sub-rede anunciada).

mod derp;

pub use derp::{DerpFake, spawn_stun};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use tokio::sync::mpsc;

use crate::control::map::encode_frame;
use crate::control::noise::{NoiseStream, TAMANHO_INICIACAO, servidor as lado_servidor};
use crate::key::{ChallengePublic, MachineKey, MachinePublic, NodeKey, NodePublic};
use crate::netmap::NetMap;
use crate::tailcfg::{
    Hostinfo, Login, MapRequest, MapResponse, Node, PeerChange, RegisterRequest, RegisterResponse,
    User,
};
use crate::upgrade::{Prefixed, Stream};

/// Como o servidor falso se comporta.
#[derive(Debug, Clone)]
pub struct FakeOptions {
    /// As chaves de autenticação aceitas.
    pub auth_keys: Vec<String>,
    /// Se um registro sem chave devolve uma URL de login (senão, devolve um erro).
    pub login_url: bool,
    /// Se, depois do login, o administrador ainda precisa aprovar a máquina.
    pub admin_approval: bool,
    /// Se o servidor manda a carga antecipada (`nodeKeyChallenge`) depois do handshake.
    pub early_payload: bool,
    /// Se `Some`, o handshake é recusado com este texto.
    pub refuse_handshake: Option<String>,
    /// Se o `/key` responde 404 (um servidor que não é do Tailscale).
    pub not_tailscale: bool,
    /// Se o registro responde que a chave do nó venceu.
    pub expire_node_key: bool,
    /// Se o registro responde com uma assinatura do Tailnet Lock.
    pub tailnet_lock: bool,
    /// A versão de capacidade mínima aceita no handshake.
    pub min_version: u16,
    /// Quanto tempo o pedido de `Followup` espera pelo login antes de devolver a URL de novo.
    pub followup_wait: Duration,
    /// Se `Some`, o registro responde com este código HTTP (e o texto) em vez do normal.
    pub http_error: Option<(u16, String)>,
    /// Se o mapa sai comprimido com `zstd` quando o cliente pede (senão, sai sempre puro).
    pub map_compress: bool,
    /// De quanto em quanto tempo o mapa em fluxo manda um sinal de vida (`None` = nunca).
    pub map_keepalive: Option<Duration>,
    /// Se o mapa de uma chave de nó que nunca se registrou dá 401 (como o Headscale).
    pub map_requires_registration: bool,
    /// Se o servidor monta o mapa de cada nó a partir dos nós que se registraram (a tailnet de
    /// verdade: cada um vê os outros como pares, com o endereço, os endpoints e o DERP de casa
    /// que declararam), em vez de devolver um mapa único dado pelo teste.
    pub tailnet: bool,
}

impl Default for FakeOptions {
    fn default() -> Self {
        FakeOptions {
            auth_keys: vec!["tskey-auth-valida".to_string()],
            login_url: true,
            admin_approval: false,
            early_payload: true,
            refuse_handshake: None,
            not_tailscale: false,
            expire_node_key: false,
            tailnet_lock: false,
            min_version: 0,
            followup_wait: Duration::from_secs(10),
            http_error: None,
            map_compress: true,
            map_keepalive: None,
            map_requires_registration: true,
            tailnet: false,
        }
    }
}

/// Um pedido de registro que o servidor recebeu.
#[derive(Debug, Clone)]
pub struct ReceivedRegister {
    /// A chave da máquina que fez o handshake.
    pub machine: MachinePublic,
    /// A versão de capacidade do handshake.
    pub handshake_version: u16,
    /// O pedido, já lido.
    pub request: RegisterRequest,
    /// O pedido como JSON, para conferir o que foi (ou não foi) escrito.
    pub raw: serde_json::Value,
}

/// Um pedido de mapa que o servidor recebeu (o fluxo ou uma atualização de endpoints).
#[derive(Debug, Clone)]
pub struct ReceivedMap {
    /// A chave da máquina que fez o handshake.
    pub machine: MachinePublic,
    /// O pedido, já lido.
    pub request: MapRequest,
    /// O pedido como JSON, para conferir o que foi (ou não foi) escrito.
    pub raw: serde_json::Value,
}

/// O que o teste manda a um fluxo de mapa aberto.
enum Mensagem {
    /// Uma resposta (um delta).
    Resposta(Vec<u8>),
    /// Encerra o fluxo.
    Fechar,
}

/// Um fluxo de mapa aberto.
struct Fluxo {
    id: u64,
    /// De que nó é (para montar o mapa dele no modo `tailnet`).
    no: NodePublic,
    tx: mpsc::UnboundedSender<Mensagem>,
    /// Os pares que o cliente já recebeu (para dizer quais saíram).
    conhecidos: HashSet<i64>,
}

/// Um nó da tailnet do modo `tailnet`.
struct Membro {
    id: i64,
    chave: NodePublic,
    disco: crate::key::DiscoPublic,
    endpoints: Vec<std::net::SocketAddr>,
    casa: i32,
    hostname: String,
}

#[derive(Default)]
struct Estado {
    /// O mapa que o servidor tem hoje; quem abre um fluxo recebe isto inteiro.
    mapa: NetMap,
    fluxos: Vec<Fluxo>,
    proximo_fluxo: u64,
    membros: Vec<Membro>,
    derp_map: crate::tailcfg::DerpMap,
    pedidos_mapa: Vec<ReceivedMap>,
    fluxos_abertos: usize,
    opcoes: Option<FakeOptions>,
    pedidos: Vec<ReceivedRegister>,
    conexoes: usize,
    handshakes_recusados: usize,
    /// URL de login → aprovada?
    logins: HashMap<String, bool>,
    /// Chaves de nó já registradas.
    registrados: HashSet<NodePublic>,
    proximo_login: u32,
}

struct Interno {
    chave: MachineKey,
    base: String,
    estado: Mutex<Estado>,
    aprovacoes: watch::Sender<u64>,
}

impl Interno {
    fn opcoes(&self) -> FakeOptions {
        self.estado
            .lock()
            .unwrap()
            .opcoes
            .clone()
            .unwrap_or_default()
    }
}

/// O plano de controle falso. Sobe ao criar e cai ao ser largado.
pub struct ControlFake {
    interno: Arc<Interno>,
    addr: std::net::SocketAddr,
    aceitador: JoinHandle<()>,
}

impl Drop for ControlFake {
    fn drop(&mut self) {
        self.aceitador.abort();
    }
}

impl ControlFake {
    /// Sobe com o comportamento padrão.
    pub async fn start() -> Self {
        Self::with_options(FakeOptions::default()).await
    }

    /// Sobe com as opções dadas.
    pub async fn with_options(opcoes: FakeOptions) -> Self {
        let escuta = TcpListener::bind("127.0.0.1:0").await.expect("porta local");
        let addr = escuta.local_addr().expect("endereço");
        let (aprov, _) = watch::channel(0u64);
        let interno = Arc::new(Interno {
            chave: MachineKey::generate(),
            base: format!("http://{addr}"),
            estado: Mutex::new(Estado {
                opcoes: Some(opcoes),
                ..Default::default()
            }),
            aprovacoes: aprov,
        });
        let i2 = interno.clone();
        let aceitador = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = escuta.accept().await else {
                    break;
                };
                let i3 = i2.clone();
                tokio::spawn(async move {
                    let _ = atender(i3, tcp).await;
                });
            }
        });
        ControlFake {
            interno,
            addr,
            aceitador,
        }
    }

    /// A URL do servidor.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// A chave pública do servidor.
    pub fn server_key(&self) -> MachinePublic {
        self.interno.chave.public()
    }

    /// Muda as opções com o servidor no ar.
    pub fn set_options(&self, f: impl FnOnce(&mut FakeOptions)) {
        let mut e = self.interno.estado.lock().unwrap();
        f(e.opcoes.get_or_insert_with(FakeOptions::default));
    }

    /// Aprova todos os logins pendentes (o usuário entrou pelo navegador).
    pub fn approve(&self) {
        {
            let mut e = self.interno.estado.lock().unwrap();
            for v in e.logins.values_mut() {
                *v = true;
            }
        }
        self.interno.aprovacoes.send_modify(|n| *n += 1);
    }

    /// As URLs de login que o servidor já entregou.
    pub fn login_urls(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .interno
            .estado
            .lock()
            .unwrap()
            .logins
            .keys()
            .cloned()
            .collect();
        v.sort();
        v
    }

    /// Os pedidos de registro recebidos, em ordem.
    pub fn requests(&self) -> Vec<ReceivedRegister> {
        self.interno.estado.lock().unwrap().pedidos.clone()
    }

    /// Quantas conexões ts2021 o servidor aceitou.
    pub fn connections(&self) -> usize {
        self.interno.estado.lock().unwrap().conexoes
    }

    /// Quantos handshakes o servidor recusou.
    pub fn refused_handshakes(&self) -> usize {
        self.interno.estado.lock().unwrap().handshakes_recusados
    }
}

impl ControlFake {
    /// Registra uma chave de nó sem passar pelo `register` (para os testes que só querem o
    /// mapa).
    pub fn register_node(&self, chave: NodePublic) {
        self.interno
            .estado
            .lock()
            .unwrap()
            .registrados
            .insert(chave);
    }

    /// Define o nosso nó no mapa e avisa os fluxos abertos.
    pub fn set_self(&self, no: Node) {
        self.push(MapResponse {
            node: Some(no),
            ..MapResponse::default()
        });
    }

    /// Acrescenta um par (ou troca o que tem o mesmo número) e avisa os fluxos abertos.
    pub fn set_peer(&self, no: Node) {
        self.push(MapResponse {
            peers_changed: vec![no],
            ..MapResponse::default()
        });
    }

    /// Tira um par e avisa os fluxos abertos.
    pub fn remove_peer(&self, id: i64) {
        self.push(MapResponse {
            peers_removed: vec![id],
            ..MapResponse::default()
        });
    }

    /// Manda uma mudança parcial de um par e avisa os fluxos abertos.
    pub fn patch_peer(&self, mudanca: PeerChange) {
        self.push(MapResponse {
            peers_changed_patch: vec![mudanca],
            ..MapResponse::default()
        });
    }

    /// Aplica a resposta no mapa do servidor e a manda, como está, a todos os fluxos abertos.
    /// Quem abrir um fluxo depois recebe o mapa inteiro já com a mudança.
    pub fn push(&self, delta: MapResponse) {
        let quadro = serde_json::to_vec(&delta).expect("serializa");
        let mut e = self.interno.estado.lock().unwrap();
        e.mapa.apply(delta);
        e.fluxos
            .retain(|f| f.tx.send(Mensagem::Resposta(quadro.clone())).is_ok());
    }

    /// Encerra todos os fluxos de mapa abertos (o cliente tem de reconectar).
    pub fn close_map_streams(&self) {
        let mut e = self.interno.estado.lock().unwrap();
        for f in e.fluxos.drain(..) {
            let _ = f.tx.send(Mensagem::Fechar);
        }
    }

    /// Quantos fluxos de mapa estão abertos agora.
    pub fn open_map_streams(&self) -> usize {
        let mut e = self.interno.estado.lock().unwrap();
        e.fluxos.retain(|f| !f.tx.is_closed());
        e.fluxos.len()
    }

    /// Define o mapa do DERP que o modo `tailnet` entrega a todos os nós.
    pub fn set_derp_map(&self, mapa: crate::tailcfg::DerpMap) {
        let mut e = self.interno.estado.lock().unwrap();
        e.derp_map = mapa;
        difundir(&mut e);
    }

    /// Os endpoints que um nó declarou ao controle (modo `tailnet`).
    pub fn declared_endpoints(&self, chave: &NodePublic) -> Vec<std::net::SocketAddr> {
        let e = self.interno.estado.lock().unwrap();
        e.membros
            .iter()
            .find(|m| m.chave == *chave)
            .map(|m| m.endpoints.clone())
            .unwrap_or_default()
    }

    /// Quantos fluxos de mapa o servidor já abriu, no total.
    pub fn map_streams_opened(&self) -> usize {
        self.interno.estado.lock().unwrap().fluxos_abertos
    }

    /// Os pedidos de mapa recebidos, em ordem (os fluxos e as atualizações de endpoints).
    pub fn map_requests(&self) -> Vec<ReceivedMap> {
        self.interno.estado.lock().unwrap().pedidos_mapa.clone()
    }
}

/// Um nó de teste: chave de nó e de disco derivadas do número, um endereço da tailnet e o
/// nome `nome.teste.ts.net.`.
pub fn fake_node(id: i64, nome: &str, ip: &str) -> Node {
    Node {
        id,
        stable_id: format!("n{id}"),
        name: format!("{nome}.teste.ts.net."),
        user: 1,
        key: NodeKey::from_bytes([id as u8; 32]).public(),
        disco_key: crate::key::DiscoKey::from_bytes([id as u8 ^ 0x80; 32]).public(),
        addresses: vec![format!("{ip}/32")],
        allowed_ips: vec![format!("{ip}/32")],
        hostinfo: Some(Hostinfo {
            hostname: nome.to_string(),
            os: "linux".to_string(),
            ..Hostinfo::default()
        }),
        machine_authorized: true,
        online: Some(true),
        ..Node::default()
    }
}

/// O pedido HTTP/1.1 que chegou: método, caminho, cabeçalhos (nomes em minúsculas) e o que
/// veio depois do cabeçalho.
struct PedidoHttp {
    metodo: String,
    caminho: String,
    cabecalhos: Vec<(String, String)>,
    sobra: Vec<u8>,
}

impl PedidoHttp {
    fn cabecalho(&self, nome: &str) -> Option<&str> {
        self.cabecalhos
            .iter()
            .find(|(n, _)| n == nome)
            .map(|(_, v)| v.as_str())
    }
}

async fn ler_pedido(tcp: &mut TcpStream) -> std::io::Result<PedidoHttp> {
    let mut buf = Vec::new();
    let mut pedaco = [0u8; 1024];
    let fim = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 16 * 1024 {
            return Err(std::io::Error::other("cabeçalho grande demais"));
        }
        let n = tcp.read(&mut pedaco).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&pedaco[..n]);
    };
    let sobra = buf.split_off(fim + 4);
    let texto = String::from_utf8_lossy(&buf[..fim]).into_owned();
    let mut linhas = texto.split("\r\n");
    let mut partes = linhas.next().unwrap_or("").split_whitespace();
    let metodo = partes.next().unwrap_or("").to_string();
    let caminho = partes.next().unwrap_or("").to_string();
    let cabecalhos = linhas
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Ok(PedidoHttp {
        metodo,
        caminho,
        cabecalhos,
        sobra,
    })
}

async fn responder_http(
    tcp: &mut TcpStream,
    status: u16,
    tipo: &str,
    corpo: &[u8],
) -> std::io::Result<()> {
    let cab = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: {tipo}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        corpo.len()
    );
    tcp.write_all(cab.as_bytes()).await?;
    tcp.write_all(corpo).await?;
    tcp.flush().await
}

async fn atender(interno: Arc<Interno>, mut tcp: TcpStream) -> std::io::Result<()> {
    let pedido = ler_pedido(&mut tcp).await?;
    let opcoes = interno.opcoes();
    let caminho = pedido.caminho.split('?').next().unwrap_or("").to_string();
    match (pedido.metodo.as_str(), caminho.as_str()) {
        ("GET", "/key") => {
            if opcoes.not_tailscale {
                return responder_http(&mut tcp, 404, "text/plain", b"nada aqui").await;
            }
            let k = interno.chave.public();
            let corpo = format!("{{\"publicKey\":\"{k}\",\"legacyPublicKey\":\"{k}\"}}");
            responder_http(&mut tcp, 200, "application/json", corpo.as_bytes()).await
        }
        ("POST", "/ts2021") => ts2021(interno, tcp, pedido, opcoes).await,
        _ => responder_http(&mut tcp, 404, "text/plain", b"nao encontrado").await,
    }
}

async fn ts2021(
    interno: Arc<Interno>,
    mut tcp: TcpStream,
    pedido: PedidoHttp,
    opcoes: FakeOptions,
) -> std::io::Result<()> {
    let b64 = pedido
        .cabecalho("x-tailscale-handshake")
        .ok_or_else(|| std::io::Error::other("sem o cabeçalho do handshake"))?;
    let bruto = STANDARD
        .decode(b64)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let iniciacao: [u8; TAMANHO_INICIACAO] = bruto
        .try_into()
        .map_err(|_| std::io::Error::other("iniciação com tamanho errado"))?;
    let aceito = match lado_servidor::accept(&interno.chave, &iniciacao, None) {
        Ok(a) => a,
        Err(e) => {
            return responder_http(&mut tcp, 400, "text/plain", e.to_string().as_bytes()).await;
        }
    };
    tcp.write_all(
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: tailscale-control-protocol\r\nConnection: upgrade\r\n\r\n",
    )
    .await?;
    let recusa = opcoes.refuse_handshake.clone().or_else(|| {
        (aceito.versao < opcoes.min_version).then(|| "unsupported client version".to_string())
    });
    if let Some(texto) = recusa {
        interno.estado.lock().unwrap().handshakes_recusados += 1;
        tcp.write_all(&lado_servidor::mensagem_de_erro(&texto))
            .await?;
        return tcp.flush().await;
    }
    tcp.write_all(&aceito.resposta).await?;
    interno.estado.lock().unwrap().conexoes += 1;
    let fluxo: Stream = Box::new(tcp);
    let fluxo = Prefixed::new(pedido.sobra, fluxo);
    let mut noise = NoiseStream::server(fluxo, aceito.transport);
    if opcoes.early_payload {
        let desafio = ChallengePublic::from_bytes([7; 32]);
        let json = format!("{{\"nodeKeyChallenge\":\"{desafio}\"}}");
        let mut m = b"\xff\xff\xffTS".to_vec();
        m.extend_from_slice(&(json.len() as u32).to_be_bytes());
        m.extend_from_slice(json.as_bytes());
        noise.write_all(&m).await?;
        noise.flush().await?;
    }
    let mut conexao = h2::server::handshake(noise)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    while let Some(r) = conexao.accept().await {
        let (req, resp) = r.map_err(|e| std::io::Error::other(e.to_string()))?;
        let i = interno.clone();
        let maquina = aceito.maquina;
        let versao = aceito.versao;
        tokio::spawn(async move {
            let _ = tratar_h2(i, req, resp, maquina, versao).await;
        });
    }
    Ok(())
}

async fn tratar_h2(
    interno: Arc<Interno>,
    req: http::Request<h2::RecvStream>,
    mut resp: h2::server::SendResponse<Bytes>,
    maquina: MachinePublic,
    versao: u16,
) -> Result<(), h2::Error> {
    let caminho = req.uri().path().to_string();
    let mut corpo = req.into_body();
    let mut bytes = Vec::new();
    while let Some(p) = corpo.data().await {
        let p = p?;
        let _ = corpo.flow_control().release_capacity(p.len());
        bytes.extend_from_slice(&p);
    }
    if caminho == "/machine/map" {
        return mapa(&interno, &bytes, maquina, resp).await;
    }
    let (status, saida) = if caminho == "/machine/register" {
        registrar(&interno, &bytes, maquina, versao).await
    } else {
        (404, b"nao encontrado".to_vec())
    };
    let r = http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(())
        .expect("resposta");
    let mut envio = resp.send_response(r, false)?;
    envio.send_data(Bytes::from(saida), true)
}

/// O `POST /machine/map`: o fluxo (o mapa inteiro e depois os deltas) ou uma atualização.
async fn mapa(
    interno: &Arc<Interno>,
    bytes: &[u8],
    maquina: MachinePublic,
    mut resp: h2::server::SendResponse<Bytes>,
) -> Result<(), h2::Error> {
    let recusa = |status: u16, texto: &str| {
        http::Response::builder()
            .status(status)
            .body(())
            .map(|r| (r, Bytes::from(texto.to_string())))
            .expect("resposta")
    };
    let Ok(pedido) = serde_json::from_slice::<MapRequest>(bytes) else {
        let (r, corpo) = recusa(400, "json invalido");
        resp.send_response(r, false)?.send_data(corpo, true)?;
        return Ok(());
    };
    let bruto: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
    let (opcoes, registrada) = {
        let mut e = interno.estado.lock().unwrap();
        e.pedidos_mapa.push(ReceivedMap {
            machine: maquina,
            request: pedido.clone(),
            raw: bruto,
        });
        (
            e.opcoes.clone().unwrap_or_default(),
            e.registrados.contains(&pedido.node_key),
        )
    };
    if opcoes.map_requires_registration && !registrada {
        let (r, corpo) = recusa(401, "node not found");
        resp.send_response(r, false)?.send_data(corpo, true)?;
        return Ok(());
    }
    let cabecalho = http::Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(())
        .expect("resposta");
    if opcoes.tailnet {
        let mut e = interno.estado.lock().unwrap();
        let hostname = pedido
            .hostinfo
            .as_ref()
            .map(|h| h.hostname.clone())
            .unwrap_or_default();
        let casa = pedido
            .hostinfo
            .as_ref()
            .and_then(|h| h.net_info.as_ref())
            .map(|n| n.preferred_derp)
            .unwrap_or(0);
        let existente = e.membros.iter().position(|m| m.chave == pedido.node_key);
        match existente {
            Some(i) => {
                let m = &mut e.membros[i];
                m.disco = pedido.disco_key;
                if !pedido.omit_peers || !pedido.endpoints.is_empty() {
                    m.endpoints = pedido.endpoints.clone();
                }
                if casa != 0 {
                    m.casa = casa;
                }
                if !hostname.is_empty() {
                    m.hostname = hostname;
                }
            }
            None => {
                let id = e.membros.len() as i64 + 1;
                e.membros.push(Membro {
                    id,
                    chave: pedido.node_key,
                    disco: pedido.disco_key,
                    endpoints: pedido.endpoints.clone(),
                    casa,
                    hostname: if hostname.is_empty() {
                        format!("no{id}")
                    } else {
                        hostname
                    },
                });
            }
        }
        if !pedido.stream {
            difundir(&mut e);
        }
    }
    if !pedido.stream {
        // A atualização de endpoints: o servidor guarda e responde vazio.
        resp.send_response(cabecalho, false)?
            .send_data(Bytes::new(), true)?;
        return Ok(());
    }
    let comprime = opcoes.map_compress && pedido.compress == "zstd";
    let (tx, mut rx) = mpsc::unbounded_channel();
    // O mapa inteiro e o registro do fluxo andam juntos, para nenhum delta se perder entre os
    // dois.
    let (inicial, _saida) = {
        let mut e = interno.estado.lock().unwrap();
        e.proximo_fluxo += 1;
        let id = e.proximo_fluxo;
        let mut conhecidos = HashSet::new();
        let inicial = if opcoes.tailnet {
            let vazio = HashSet::new();
            match resposta_para(&e, &pedido.node_key, &vazio) {
                Some((r, ids)) => {
                    conhecidos = ids;
                    r
                }
                None => MapResponse::default(),
            }
        } else {
            let m = &e.mapa;
            MapResponse {
                node: m.me().cloned(),
                peers: m.peers().cloned().collect(),
                derp_map: Some(m.derp_map().clone()),
                dns_config: Some(m.dns().clone()),
                domain: m.domain().to_string(),
                ..MapResponse::default()
            }
        };
        e.fluxos.push(Fluxo {
            id,
            no: pedido.node_key,
            tx,
            conhecidos,
        });
        e.fluxos_abertos += 1;
        if opcoes.tailnet {
            // Os outros nós passam a ver este como conectado.
            let outros: Vec<usize> = (0..e.fluxos.len())
                .filter(|i| e.fluxos[*i].id != id)
                .collect();
            let mut envios = Vec::new();
            for i in outros {
                let f = &e.fluxos[i];
                if let Some((r, ids)) = resposta_para(&e, &f.no, &f.conhecidos) {
                    envios.push((i, serde_json::to_vec(&r).expect("serializa"), ids));
                }
            }
            for (i, json, ids) in envios {
                let f = &mut e.fluxos[i];
                if f.tx.send(Mensagem::Resposta(json)).is_ok() {
                    f.conhecidos = ids;
                }
            }
        }
        (
            inicial,
            SaidaDoFluxo {
                interno: interno.clone(),
                id,
            },
        )
    };
    let mut envio = resp.send_response(cabecalho, false)?;
    let json = serde_json::to_vec(&inicial).expect("serializa");
    envio.send_data(Bytes::from(encode_frame(&json, comprime)), false)?;
    let sinal = encode_frame(br#"{"KeepAlive":true}"#, comprime);
    loop {
        let espera = async {
            match opcoes.map_keepalive {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            m = rx.recv() => match m {
                Some(Mensagem::Resposta(json)) => {
                    envio.send_data(Bytes::from(encode_frame(&json, comprime)), false)?;
                }
                Some(Mensagem::Fechar) | None => {
                    envio.send_data(Bytes::new(), true)?;
                    return Ok(());
                }
            },
            _ = espera => {
                envio.send_data(Bytes::from(sinal.clone()), false)?;
            }
        }
    }
}

/// O endereço da tailnet de um nó do modo `tailnet`.
fn endereco_do_membro(id: i64) -> String {
    format!("100.64.0.{id}")
}

fn no_do_membro(m: &Membro, online: bool) -> Node {
    let ip = endereco_do_membro(m.id);
    Node {
        id: m.id,
        stable_id: format!("n{}", m.id),
        name: format!("{}.teste.ts.net.", m.hostname),
        user: 1,
        key: m.chave,
        disco_key: m.disco,
        addresses: vec![format!("{ip}/32")],
        allowed_ips: vec![format!("{ip}/32")],
        endpoints: m.endpoints.clone(),
        home_derp: m.casa,
        hostinfo: Some(Hostinfo {
            hostname: m.hostname.clone(),
            os: "linux".to_string(),
            ..Hostinfo::default()
        }),
        machine_authorized: true,
        online: Some(online),
        ..Node::default()
    }
}

/// O mapa de um nó no modo `tailnet`: ele mesmo, os outros como pares e o DERP.
fn resposta_para(
    e: &Estado,
    no: &NodePublic,
    conhecidos: &HashSet<i64>,
) -> Option<(MapResponse, HashSet<i64>)> {
    let eu = e.membros.iter().find(|m| m.chave == *no)?;
    let online = |m: &Membro| e.fluxos.iter().any(|f| f.no == m.chave);
    let pares: Vec<Node> = e
        .membros
        .iter()
        .filter(|m| m.chave != *no)
        .map(|m| no_do_membro(m, online(m)))
        .collect();
    let ids: HashSet<i64> = pares.iter().map(|n| n.id).collect();
    let mut saiu: Vec<i64> = conhecidos.difference(&ids).copied().collect();
    saiu.sort_unstable();
    let resposta = MapResponse {
        node: Some(no_do_membro(eu, true)),
        peers: pares,
        peers_removed: saiu,
        derp_map: Some(e.derp_map.clone()),
        dns_config: Some(crate::tailcfg::DnsConfig::default()),
        domain: "teste.ts.net".to_string(),
        ..MapResponse::default()
    };
    Some((resposta, ids))
}

/// Manda o mapa de cada nó (o do modo `tailnet`) aos fluxos abertos.
fn difundir(e: &mut Estado) {
    let opcoes_tailnet = e.opcoes.as_ref().is_some_and(|o| o.tailnet);
    if !opcoes_tailnet {
        return;
    }
    let mut envios = Vec::new();
    for (i, f) in e.fluxos.iter().enumerate() {
        if let Some((r, ids)) = resposta_para(e, &f.no, &f.conhecidos) {
            envios.push((i, serde_json::to_vec(&r).expect("serializa"), ids));
        }
    }
    for (i, json, ids) in envios {
        let f = &mut e.fluxos[i];
        if f.tx.send(Mensagem::Resposta(json)).is_ok() {
            f.conhecidos = ids;
        }
    }
}

/// Tira o fluxo quando a conexão acaba (e avisa os outros nós de que ele ficou offline).
struct SaidaDoFluxo {
    interno: Arc<Interno>,
    id: u64,
}

impl Drop for SaidaDoFluxo {
    fn drop(&mut self) {
        let mut e = self.interno.estado.lock().unwrap();
        e.fluxos.retain(|f| f.id != self.id);
        difundir(&mut e);
    }
}

fn json<T: serde::Serialize>(v: &T) -> (u16, Vec<u8>) {
    (200, serde_json::to_vec(v).expect("serializa"))
}

async fn registrar(
    interno: &Arc<Interno>,
    bytes: &[u8],
    maquina: MachinePublic,
    versao: u16,
) -> (u16, Vec<u8>) {
    let Ok(pedido) = serde_json::from_slice::<RegisterRequest>(bytes) else {
        return (400, b"json invalido".to_vec());
    };
    let bruto: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
    let opcoes = {
        let mut e = interno.estado.lock().unwrap();
        e.pedidos.push(ReceivedRegister {
            machine: maquina,
            handshake_version: versao,
            request: pedido.clone(),
            raw: bruto,
        });
        e.opcoes.clone().unwrap_or_default()
    };
    if let Some((codigo, texto)) = opcoes.http_error {
        return (codigo, texto.into_bytes());
    }
    if opcoes.tailnet_lock {
        return json(&RegisterResponse {
            node_key_signature: Some("AAAA".to_string()),
            ..Default::default()
        });
    }
    if opcoes.expire_node_key {
        return json(&RegisterResponse {
            node_key_expired: true,
            ..Default::default()
        });
    }
    let autorizada = |aprovada: bool| RegisterResponse {
        user: User {
            id: 1,
            display_name: "Teste".to_string(),
        },
        login: Login::default(),
        machine_authorized: aprovada,
        ..Default::default()
    };
    // Com chave de autenticação.
    if let Some(auth) = &pedido.auth
        && !auth.auth_key.is_empty()
    {
        if opcoes.auth_keys.contains(&auth.auth_key) {
            interno
                .estado
                .lock()
                .unwrap()
                .registrados
                .insert(pedido.node_key);
            return json(&autorizada(true));
        }
        return json(&RegisterResponse {
            error: "invalid key: unable to validate API key".to_string(),
            ..Default::default()
        });
    }
    // Segundo pedido: o usuário está entrando pelo navegador.
    if !pedido.followup.is_empty() {
        let url = pedido.followup.clone();
        let mut rx = interno.aprovacoes.subscribe();
        let espera = tokio::time::timeout(opcoes.followup_wait, async {
            loop {
                let aprovado = interno
                    .estado
                    .lock()
                    .unwrap()
                    .logins
                    .get(&url)
                    .copied()
                    .unwrap_or(false);
                if aprovado {
                    return;
                }
                if rx.changed().await.is_err() {
                    return;
                }
            }
        })
        .await;
        return match espera {
            Ok(()) => {
                interno
                    .estado
                    .lock()
                    .unwrap()
                    .registrados
                    .insert(pedido.node_key);
                json(&autorizada(!opcoes.admin_approval))
            }
            // Ninguém entrou a tempo: devolve a mesma URL, como um servidor que repete.
            Err(_) => json(&RegisterResponse {
                auth_url: url,
                ..Default::default()
            }),
        };
    }
    // Já registrada antes (a mesma chave de nó): entra direto.
    if interno
        .estado
        .lock()
        .unwrap()
        .registrados
        .contains(&pedido.node_key)
    {
        return json(&autorizada(true));
    }
    if opcoes.login_url {
        let url = {
            let mut e = interno.estado.lock().unwrap();
            e.proximo_login += 1;
            let url = format!("{}/a/{:08x}", interno.base, e.proximo_login);
            e.logins.insert(url.clone(), false);
            url
        };
        return json(&RegisterResponse {
            auth_url: url,
            ..Default::default()
        });
    }
    json(&RegisterResponse {
        error: "a chave de autenticação é necessária".to_string(),
        ..Default::default()
    })
}
