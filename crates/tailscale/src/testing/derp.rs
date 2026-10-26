//! O servidor DERP falso e o servidor STUN falso dos testes.
//!
//! O [`DerpFake`] fala o DERP de verdade em HTTP puro na porta local (`GET /derp` com
//! `Upgrade: DERP`): manda a chave do servidor, confere a apresentação do cliente (a caixa NaCl),
//! responde com o `ServerInfo` e repassa os pacotes entre os clientes conectados. Guarda o que
//! viu, para os testes conferirem.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use tomni_ui_netstack::Underlay;

use crate::derp::{MAGIC, encode_frame, read_frame, tipo};
use crate::key::{NodeKey, NodePublic};
use crate::nacl::NaclBox;
use crate::stun;
use crate::tailcfg::{DerpMap, DerpNode, DerpRegion};

struct Cliente {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    /// Para derrubar a conexão a pedido do teste.
    derrubar: mpsc::UnboundedSender<()>,
}

#[derive(Default)]
struct Estado {
    clientes: HashMap<NodePublic, Cliente>,
    preferidos: HashMap<NodePublic, bool>,
    pacotes: u64,
    conexoes: u64,
    recusadas: u64,
    descartar: bool,
}

struct Interno {
    chave: NodeKey,
    estado: Mutex<Estado>,
}

/// O servidor DERP falso. Sobe ao criar e cai ao ser largado.
pub struct DerpFake {
    interno: Arc<Interno>,
    addr: SocketAddr,
    aceitador: JoinHandle<()>,
}

impl Drop for DerpFake {
    fn drop(&mut self) {
        self.aceitador.abort();
        let e = self.interno.estado.lock().unwrap();
        for c in e.clientes.values() {
            let _ = c.derrubar.send(());
        }
    }
}

impl DerpFake {
    /// Sobe numa porta local.
    pub async fn start() -> Self {
        let escuta = TcpListener::bind("127.0.0.1:0").await.expect("porta local");
        let addr = escuta.local_addr().expect("endereço");
        let interno = Arc::new(Interno {
            chave: NodeKey::generate(),
            estado: Mutex::new(Estado::default()),
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
        DerpFake {
            interno,
            addr,
            aceitador,
        }
    }

    /// O endereço local do servidor.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// A chave pública do servidor.
    pub fn server_key(&self) -> NodePublic {
        self.interno.chave.public()
    }

    /// O servidor como o mapa do controle o descreve (um nó, HTTP puro).
    pub fn node(&self, regiao: i32) -> DerpNode {
        DerpNode {
            name: format!("{regiao}a"),
            region_id: regiao,
            host_name: "127.0.0.1".to_string(),
            ipv4: "127.0.0.1".to_string(),
            ipv6: "none".to_string(),
            stun_port: -1,
            derp_port: i32::from(self.addr.port()),
            insecure_for_tests: true,
            ..DerpNode::default()
        }
    }

    /// A região do mapa com este servidor.
    pub fn region(&self, regiao: i32, nome: &str) -> DerpRegion {
        DerpRegion {
            region_id: regiao,
            region_code: nome.to_string(),
            region_name: nome.to_string(),
            nodes: vec![self.node(regiao)],
            ..DerpRegion::default()
        }
    }

    /// Um mapa do DERP com esta única região.
    pub fn derp_map(&self, regiao: i32) -> DerpMap {
        let mut mapa = DerpMap {
            omit_default_regions: true,
            ..DerpMap::default()
        };
        mapa.regions
            .insert(regiao, Some(self.region(regiao, "teste")));
        mapa
    }

    /// Quantos clientes estão conectados agora.
    pub fn connected(&self) -> usize {
        self.interno.estado.lock().unwrap().clientes.len()
    }

    /// Se o nó está conectado agora.
    pub fn is_connected(&self, chave: &NodePublic) -> bool {
        self.interno
            .estado
            .lock()
            .unwrap()
            .clientes
            .contains_key(chave)
    }

    /// Se o nó disse que este servidor é a casa dele.
    pub fn is_preferred(&self, chave: &NodePublic) -> bool {
        self.interno
            .estado
            .lock()
            .unwrap()
            .preferidos
            .get(chave)
            .copied()
            .unwrap_or(false)
    }

    /// Quantos pacotes o servidor repassou (ou tentou repassar).
    pub fn relayed_packets(&self) -> u64 {
        self.interno.estado.lock().unwrap().pacotes
    }

    /// Quantas conexões já aceitou, no total.
    pub fn connections(&self) -> u64 {
        self.interno.estado.lock().unwrap().conexoes
    }

    /// Quantas apresentações recusou (caixa que não abriu).
    pub fn refused(&self) -> u64 {
        self.interno.estado.lock().unwrap().recusadas
    }

    /// Derruba todas as conexões (o cliente tem de reconectar).
    pub fn drop_connections(&self) {
        let mut e = self.interno.estado.lock().unwrap();
        for (_, c) in e.clientes.drain() {
            let _ = c.derrubar.send(());
        }
    }

    /// Liga ou desliga o descarte silencioso dos pacotes (um DERP que não entrega nada).
    pub fn set_discard(&self, sim: bool) {
        self.interno.estado.lock().unwrap().descartar = sim;
    }
}

async fn atender(interno: Arc<Interno>, mut tcp: TcpStream) -> std::io::Result<()> {
    let _ = tcp.set_nodelay(true);
    // O pedido HTTP: lê até o fim do cabeçalho.
    let mut buf = Vec::new();
    let mut pedaco = [0u8; 512];
    loop {
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        let n = tcp.read(&mut pedaco).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&pedaco[..n]);
        if buf.len() > 16 * 1024 {
            return Ok(());
        }
    }
    let texto = String::from_utf8_lossy(&buf).to_ascii_lowercase();
    if !texto.starts_with("get /derp ") || !texto.contains("upgrade: derp") {
        tcp.write_all(b"HTTP/1.1 404 X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(());
    }
    tcp.write_all(
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: DERP\r\nConnection: Upgrade\r\n\r\n",
    )
    .await?;
    let mut chave_servidor = MAGIC.to_vec();
    chave_servidor.extend_from_slice(interno.chave.public().as_bytes());
    tcp.write_all(&encode_frame(tipo::SERVER_KEY, &[&chave_servidor]))
        .await?;
    let (mut leitura, mut escrita) = tcp.into_split();
    // A apresentação do cliente.
    let (t, carga) = read_frame(&mut leitura).await?;
    if t != tipo::CLIENT_INFO || carga.len() < 32 + 24 {
        return Ok(());
    }
    let cliente = NodePublic::from_bytes(carga[..32].try_into().expect("32 bytes"));
    let caixa = NaclBox::new(&interno.chave.to_bytes(), cliente.as_bytes());
    if caixa.open(&carga[32..]).is_none() {
        interno.estado.lock().unwrap().recusadas += 1;
        return Ok(());
    }
    let info = caixa.seal(br#"{"version":2}"#);
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (derrubar_tx, mut derrubar_rx) = mpsc::unbounded_channel::<()>();
    {
        let mut e = interno.estado.lock().unwrap();
        e.conexoes += 1;
        e.clientes.insert(
            cliente,
            Cliente {
                tx: tx.clone(),
                derrubar: derrubar_tx,
            },
        );
    }
    let _ = tx.send(encode_frame(tipo::SERVER_INFO, &[&info]));
    let escritor = tokio::spawn(async move {
        while let Some(q) = rx.recv().await {
            if escrita.write_all(&q).await.is_err() || escrita.flush().await.is_err() {
                break;
            }
        }
    });
    let resultado = async {
        loop {
            let leitura_fut = read_frame(&mut leitura);
            tokio::select! {
                q = leitura_fut => {
                    let (t, carga) = q?;
                    match t {
                        tipo::SEND_PACKET if carga.len() > 32 => {
                            let destino = NodePublic::from_bytes(carga[..32].try_into().expect("32"));
                            let mut e = interno.estado.lock().unwrap();
                            e.pacotes += 1;
                            if e.descartar {
                                continue;
                            }
                            let quadro = encode_frame(tipo::RECV_PACKET, &[cliente.as_bytes(), &carga[32..]]);
                            match e.clientes.get(&destino) {
                                Some(c) => { let _ = c.tx.send(quadro); }
                                None => {
                                    let gone = encode_frame(tipo::PEER_GONE, &[destino.as_bytes(), &[1u8]]);
                                    let _ = tx.send(gone);
                                }
                            }
                        }
                        tipo::NOTE_PREFERRED if !carga.is_empty() => {
                            interno
                                .estado
                                .lock()
                                .unwrap()
                                .preferidos
                                .insert(cliente, carga[0] != 0);
                        }
                        tipo::PING if carga.len() >= 8 => {
                            let _ = tx.send(encode_frame(tipo::PONG, &[&carga[..8]]));
                        }
                        _ => {}
                    }
                }
                _ = derrubar_rx.recv() => {
                    return Ok::<(), std::io::Error>(());
                }
            }
        }
    }
    .await;
    {
        let mut e = interno.estado.lock().unwrap();
        // Só tira se ainda é esta mesma conexão (o cliente pode ter reconectado).
        if e.clientes
            .get(&cliente)
            .is_some_and(|c| c.tx.same_channel(&tx))
        {
            e.clientes.remove(&cliente);
        }
    }
    escritor.abort();
    resultado
}

/// Um servidor STUN falso sobre um [`Underlay`] (por exemplo, um endpoint da rede simulada):
/// responde a cada pedido `Binding` com o endereço de onde ele veio. Devolve a tarefa; largar o
/// `JoinHandle` não a encerra, aborte-a.
pub fn spawn_stun<U: Underlay>(under: Arc<U>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = vec![0u8; 2048];
        loop {
            let recebido = std::future::poll_fn(|cx| under.poll_recv_from(cx, &mut buf)).await;
            let Ok((n, de)) = recebido else {
                return;
            };
            if !stun::looks_like_stun(&buf[..n]) || n < 20 {
                continue;
            }
            let tx: stun::TxId = buf[8..20].try_into().expect("12 bytes");
            let resposta = stun::response(&tx, de);
            if !resposta.is_empty() {
                let _ = under.try_send_to(&resposta, de);
            }
        }
    })
}
