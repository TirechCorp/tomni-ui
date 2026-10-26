//! O cliente do DERP: o relé do Tailscale, que carrega os pacotes WireGuard e do disco entre dois
//! nós quando eles não se alcançam direto. É a rota que sempre funciona (só precisa de HTTPS de
//! saída).
//!
//! A conexão é um `GET /derp` com `Upgrade: DERP`; depois dele vêm quadros
//! `[tipo u8][tamanho u32 big-endian][carga]`. O servidor abre com a chave dele, o cliente se
//! apresenta com a chave de nó e uma caixa NaCl, e daí em diante `SendPacket` leva um pacote
//! para outra chave de nó e `RecvPacket` traz o que outro nó mandou para a nossa. Portado do
//! Tailscale (github.com/tailscale/tailscale, `derp/derp.go` e `derp/derphttp`, licença
//! BSD-3-Clause).
//!
//! O [`Client`] é uma tarefa que conecta, reconecta com espera crescente e fala com o resto
//! por canais; quem decide por qual região mandar é o [`regions::Regions`].

pub mod regions;

use std::net::IpAddr;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::control::Backoff;
use crate::key::{KEY_LEN, NodeKey, NodePublic};
use crate::nacl::NaclBox;
use crate::tailcfg::{DerpNode, DerpRegionId};
use crate::upgrade::{Roots, Target, upgrade_with};

/// O maior pacote que o DERP carrega.
pub const MAX_PACKET: usize = 64 * 1024;
/// O maior quadro aceito (o pacote, a chave e uma folga para os quadros de controle).
const MAX_FRAME: usize = MAX_PACKET + 4096;
/// A marca que abre o quadro com a chave do servidor: `DERP` e o emoji de chave.
pub const MAGIC: [u8; 8] = *b"DERP\xf0\x9f\x94\x91";

/// Os tipos de quadro do DERP.
pub mod tipo {
    /// A chave do servidor (servidor para cliente).
    pub const SERVER_KEY: u8 = 0x01;
    /// A apresentação do cliente.
    pub const CLIENT_INFO: u8 = 0x02;
    /// A resposta do servidor à apresentação.
    pub const SERVER_INFO: u8 = 0x03;
    /// Um pacote para outro nó.
    pub const SEND_PACKET: u8 = 0x04;
    /// Um pacote vindo de outro nó.
    pub const RECV_PACKET: u8 = 0x05;
    /// Só para manter a conexão viva.
    pub const KEEP_ALIVE: u8 = 0x06;
    /// "Esta é a minha região de casa" (um byte: 1 sim, 0 não).
    pub const NOTE_PREFERRED: u8 = 0x07;
    /// O outro nó saiu do servidor (ou nunca esteve).
    pub const PEER_GONE: u8 = 0x08;
    /// Ping (8 bytes).
    pub const PING: u8 = 0x12;
    /// Pong (os mesmos 8 bytes).
    pub const PONG: u8 = 0x13;
    /// Um aviso de saúde do servidor (texto; vazio = tudo certo).
    pub const HEALTH: u8 = 0x14;
    /// O servidor vai reiniciar.
    pub const RESTARTING: u8 = 0x15;
}

/// Quanto tempo sem receber nada até a conexão ser dada como morta.
const OCIOSO_MAXIMO: Duration = Duration::from_secs(60);
/// De quanto em quanto tempo mandamos um ping (para o servidor responder e provar que vive).
const INTERVALO_PING: Duration = Duration::from_secs(25);
/// O tempo máximo de uma escrita (um servidor travado não pode prender a tarefa).
const TEMPO_ESCRITA: Duration = Duration::from_secs(15);
/// Depois de quanto tempo conectada a conexão é "boa" e a espera volta ao começo.
const CONEXAO_BOA: Duration = Duration::from_secs(5);
/// A fila de pacotes para o servidor: cheia, o pacote se perde (como no UDP).
const FILA: usize = 512;

/// Onde e como falar com um servidor DERP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerpTarget {
    /// A região.
    pub region: DerpRegionId,
    /// O nome do servidor (para as mensagens).
    pub name: String,
    /// O nome do certificado (o `HostName` se não houver `CertName`).
    pub host: String,
    /// O IP fixo do mapa, quando há.
    pub ip: Option<IpAddr>,
    /// A porta.
    pub port: u16,
    /// Se é HTTPS. Só o `InsecureForTests` do mapa fala HTTP puro (o servidor falso dos testes).
    pub https: bool,
}

impl DerpTarget {
    /// O alvo de um servidor do mapa; `None` se ele só atende STUN.
    pub fn from_node(no: &DerpNode) -> Option<Self> {
        if no.stun_only {
            return None;
        }
        let ip = match no.ipv4.as_str() {
            "" | "none" => None,
            texto => texto.parse::<IpAddr>().ok(),
        };
        let host = if no.host_name.is_empty() {
            ip?.to_string()
        } else if no.cert_name.is_empty() {
            no.host_name.clone()
        } else {
            no.cert_name.clone()
        };
        let port = match no.derp_port {
            p if p > 0 && p <= i32::from(u16::MAX) => p as u16,
            _ => 443,
        };
        Some(DerpTarget {
            region: no.region_id,
            name: no.name.clone(),
            host,
            ip,
            port,
            https: !no.insecure_for_tests,
        })
    }

    fn target(&self) -> Target {
        Target {
            https: self.https,
            host: self.host.clone(),
            port: self.port,
        }
    }
}

/// O que um servidor DERP conta ao cliente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A conexão está pronta (o `ServerInfo` chegou).
    Connected,
    /// Um pacote de outro nó.
    Packet {
        /// Quem mandou.
        from: NodePublic,
        /// O pacote (WireGuard ou disco).
        data: Vec<u8>,
    },
    /// O servidor diz que o nó não está (mais) nele.
    PeerGone(NodePublic),
    /// Um aviso de saúde do servidor (vazio = resolvido).
    Health(String),
    /// A conexão caiu; o cliente já está tentando de novo.
    Disconnected(String),
}

/// Um erro da conversa com o servidor.
#[derive(Debug)]
enum Falha {
    Conexao(String),
    Protocolo(String),
}

impl std::fmt::Display for Falha {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Falha::Conexao(m) => write!(f, "{m}"),
            Falha::Protocolo(m) => write!(f, "o servidor DERP fez algo inesperado: {m}"),
        }
    }
}

impl From<std::io::Error> for Falha {
    fn from(e: std::io::Error) -> Self {
        Falha::Conexao(e.to_string())
    }
}

/// Lê um quadro: o tipo e a carga.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> std::io::Result<(u8, Vec<u8>)> {
    let mut cab = [0u8; 5];
    r.read_exact(&mut cab).await?;
    let tamanho = u32::from_be_bytes([cab[1], cab[2], cab[3], cab[4]]) as usize;
    if tamanho > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("quadro de {tamanho} bytes é grande demais"),
        ));
    }
    let mut carga = vec![0u8; tamanho];
    r.read_exact(&mut carga).await?;
    Ok((cab[0], carga))
}

/// Monta um quadro.
pub fn encode_frame(tipo: u8, partes: &[&[u8]]) -> Vec<u8> {
    let total: usize = partes.iter().map(|p| p.len()).sum();
    let mut v = Vec::with_capacity(5 + total);
    v.push(tipo);
    v.extend_from_slice(&(total as u32).to_be_bytes());
    for p in partes {
        v.extend_from_slice(p);
    }
    v
}

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, quadro: &[u8]) -> Result<(), Falha> {
    match tokio::time::timeout(TEMPO_ESCRITA, async {
        w.write_all(quadro).await?;
        w.flush().await
    })
    .await
    {
        Ok(r) => r.map_err(Falha::from),
        Err(_) => Err(Falha::Conexao("o servidor DERP parou de ler".to_string())),
    }
}

/// A apresentação do cliente: a chave de nó e a caixa com o JSON de capacidades.
pub fn client_info(chave: &NodeKey, servidor: &NodePublic) -> Vec<u8> {
    let json = br#"{"version":2,"CanAckPings":true}"#;
    let caixa = NaclBox::new(&chave.to_bytes(), servidor.as_bytes());
    let mut v = chave.public().as_bytes().to_vec();
    v.extend_from_slice(&caixa.seal(json));
    v
}

enum Saida {
    Pacote(NodePublic, Vec<u8>),
    Preferida(bool),
}

/// A conexão com um servidor DERP, mantida por uma tarefa.
///
/// Largar o `Client` encerra a tarefa.
pub struct Client {
    tx: mpsc::Sender<Saida>,
    tarefa: JoinHandle<()>,
    target: DerpTarget,
}

impl Client {
    /// Começa a conectar. Os eventos vão para `eventos`, marcados com a região.
    pub fn spawn(
        target: DerpTarget,
        chave: NodeKey,
        raizes: Roots,
        preferida: bool,
        eventos: mpsc::UnboundedSender<(DerpRegionId, Event)>,
    ) -> Client {
        let (tx, rx) = mpsc::channel(FILA);
        let t = target.clone();
        let tarefa = tokio::spawn(async move {
            laco(t, chave, raizes, preferida, rx, eventos).await;
        });
        Client { tx, tarefa, target }
    }

    /// O servidor a que este cliente se liga.
    pub fn target(&self) -> &DerpTarget {
        &self.target
    }

    /// Manda um pacote para outro nó. `false` se a fila está cheia (o pacote se perde).
    pub fn send(&self, para: NodePublic, pacote: Vec<u8>) -> bool {
        if pacote.len() > MAX_PACKET {
            return false;
        }
        self.tx.try_send(Saida::Pacote(para, pacote)).is_ok()
    }

    /// Diz ao servidor se esta é a nossa região de casa.
    pub fn note_preferred(&self, sim: bool) {
        let _ = self.tx.try_send(Saida::Preferida(sim));
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.tarefa.abort();
    }
}

async fn laco(
    alvo: DerpTarget,
    chave: NodeKey,
    raizes: Roots,
    mut preferida: bool,
    mut rx: mpsc::Receiver<Saida>,
    eventos: mpsc::UnboundedSender<(DerpRegionId, Event)>,
) {
    let mut espera = Backoff::new(Duration::from_millis(500), Duration::from_secs(15));
    loop {
        // O que ficou na fila de uma conexão anterior já é velho; só o estado "preferida" vale.
        while let Ok(s) = rx.try_recv() {
            if let Saida::Preferida(p) = s {
                preferida = p;
            }
        }
        let inicio = Instant::now();
        let r = sessao(&alvo, &chave, &raizes, &mut preferida, &mut rx, &eventos).await;
        let motivo = match r {
            Ok(()) => return,
            Err(f) => f.to_string(),
        };
        if eventos
            .send((alvo.region, Event::Disconnected(motivo)))
            .is_err()
        {
            return;
        }
        if inicio.elapsed() >= CONEXAO_BOA {
            espera.reset();
        }
        tokio::time::sleep(espera.next_delay()).await;
    }
}

/// Uma conexão, da abertura à queda. `Ok(())` quando quem usa largou o cliente.
async fn sessao(
    alvo: &DerpTarget,
    chave: &NodeKey,
    raizes: &Roots,
    preferida: &mut bool,
    rx: &mut mpsc::Receiver<Saida>,
    eventos: &mpsc::UnboundedSender<(DerpRegionId, Event)>,
) -> Result<(), Falha> {
    let regiao = alvo.region;
    let mut fluxo = upgrade_with(&alvo.target(), raizes, alvo.ip, "GET", "/derp", "DERP", &[])
        .await
        .map_err(|e| Falha::Conexao(e.to_string()))?;
    // O servidor abre com a chave dele.
    let (t, carga) = tokio::time::timeout(Duration::from_secs(15), read_frame(&mut fluxo))
        .await
        .map_err(|_| Falha::Conexao("o servidor DERP não se apresentou".to_string()))??;
    if t != tipo::SERVER_KEY || carga.len() < MAGIC.len() + KEY_LEN || carga[..MAGIC.len()] != MAGIC
    {
        return Err(Falha::Protocolo("a chave do servidor não veio".to_string()));
    }
    let chave_servidor = NodePublic::from_bytes(
        carga[MAGIC.len()..MAGIC.len() + KEY_LEN]
            .try_into()
            .map_err(|_| Falha::Protocolo("chave curta".to_string()))?,
    );
    write_frame(
        &mut fluxo,
        &encode_frame(tipo::CLIENT_INFO, &[&client_info(chave, &chave_servidor)]),
    )
    .await?;
    let (mut leitura, mut escrita) = tokio::io::split(fluxo);
    // A leitura vai numa tarefa à parte: `read_exact` não pode ser cancelado no `select!`.
    let (quadros_tx, mut quadros) = mpsc::channel::<std::io::Result<(u8, Vec<u8>)>>(64);
    let leitor = tokio::spawn(async move {
        loop {
            let q = read_frame(&mut leitura).await;
            let fim = q.is_err();
            if quadros_tx.send(q).await.is_err() || fim {
                return;
            }
        }
    });
    let caixa = NaclBox::new(&chave.to_bytes(), chave_servidor.as_bytes());
    let resultado = rodar(
        regiao,
        &caixa,
        &mut escrita,
        &mut quadros,
        preferida,
        rx,
        eventos,
    )
    .await;
    leitor.abort();
    resultado
}

async fn rodar<W: AsyncWrite + Unpin>(
    regiao: DerpRegionId,
    caixa: &NaclBox,
    escrita: &mut W,
    quadros: &mut mpsc::Receiver<std::io::Result<(u8, Vec<u8>)>>,
    preferida: &mut bool,
    rx: &mut mpsc::Receiver<Saida>,
    eventos: &mpsc::UnboundedSender<(DerpRegionId, Event)>,
) -> Result<(), Falha> {
    let mut pronta = false;
    let mut ultimo_recebido = Instant::now();
    let mut proximo_ping = Instant::now() + INTERVALO_PING;
    let mut contador: u64 = 0;
    loop {
        tokio::select! {
            q = quadros.recv() => {
                let (t, carga) = match q {
                    Some(Ok(q)) => q,
                    Some(Err(e)) => return Err(Falha::Conexao(e.to_string())),
                    None => return Err(Falha::Conexao("a conexão fechou".to_string())),
                };
                ultimo_recebido = Instant::now();
                match t {
                    tipo::SERVER_INFO => {
                        if caixa.open(&carga).is_none() {
                            return Err(Falha::Protocolo(
                                "o ServerInfo não abriu com a chave do servidor".to_string(),
                            ));
                        }
                        if !pronta {
                            pronta = true;
                            if *preferida {
                                let q = encode_frame(tipo::NOTE_PREFERRED, &[&[1u8]]);
                                write_frame(escrita, &q).await?;
                            }
                            let _ = eventos.send((regiao, Event::Connected));
                        }
                    }
                    tipo::RECV_PACKET if carga.len() > KEY_LEN => {
                        let de: [u8; KEY_LEN] = carga[..KEY_LEN].try_into().expect("32 bytes");
                        let _ = eventos.send((regiao, Event::Packet {
                            from: NodePublic::from_bytes(de),
                            data: carga[KEY_LEN..].to_vec(),
                        }));
                    }
                    tipo::PEER_GONE if carga.len() >= KEY_LEN => {
                        let de: [u8; KEY_LEN] = carga[..KEY_LEN].try_into().expect("32 bytes");
                        let _ = eventos.send((regiao, Event::PeerGone(NodePublic::from_bytes(de))));
                    }
                    tipo::PING if carga.len() >= 8 => {
                        write_frame(escrita, &encode_frame(tipo::PONG, &[&carga[..8]])).await?;
                    }
                    tipo::HEALTH => {
                        let texto = String::from_utf8_lossy(&carga).into_owned();
                        let _ = eventos.send((regiao, Event::Health(texto)));
                    }
                    tipo::RESTARTING => {
                        return Err(Falha::Conexao("o servidor DERP vai reiniciar".to_string()));
                    }
                    // KeepAlive, Pong e o que não conhecemos: só provam que está vivo.
                    _ => {}
                }
            }
            s = rx.recv(), if pronta => {
                match s {
                    None => return Ok(()),
                    Some(Saida::Pacote(para, dados)) => {
                        let q = encode_frame(tipo::SEND_PACKET, &[para.as_bytes(), &dados]);
                        write_frame(escrita, &q).await?;
                    }
                    Some(Saida::Preferida(p)) => {
                        *preferida = p;
                        let q = encode_frame(tipo::NOTE_PREFERRED, &[&[u8::from(p)]]);
                        write_frame(escrita, &q).await?;
                    }
                }
            }
            _ = tokio::time::sleep_until(proximo_ping) => {
                proximo_ping = Instant::now() + INTERVALO_PING;
                if ultimo_recebido.elapsed() > OCIOSO_MAXIMO {
                    return Err(Falha::Conexao("o servidor DERP parou de responder".to_string()));
                }
                contador += 1;
                let q = encode_frame(tipo::PING, &[&contador.to_be_bytes()]);
                write_frame(escrita, &q).await?;
            }
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn o_quadro_tem_tipo_e_tamanho_big_endian() {
        let q = encode_frame(tipo::SEND_PACKET, &[&[1, 2, 3], &[4, 5]]);
        assert_eq!(q, vec![4, 0, 0, 0, 5, 1, 2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn le_o_quadro_e_recusa_o_gigante() {
        let bytes = encode_frame(tipo::KEEP_ALIVE, &[]);
        let (t, c) = read_frame(&mut &bytes[..]).await.unwrap();
        assert_eq!((t, c.len()), (tipo::KEEP_ALIVE, 0));
        let gigante = [tipo::RECV_PACKET, 0x10, 0, 0, 0];
        assert!(read_frame(&mut &gigante[..]).await.is_err());
        let cortado = [tipo::RECV_PACKET, 0, 0, 0, 9, 1];
        assert!(read_frame(&mut &cortado[..]).await.is_err());
    }

    #[test]
    fn o_alvo_vem_do_no_do_mapa() {
        let mut no = DerpNode {
            name: "1a".into(),
            region_id: 1,
            host_name: "derp1.exemplo.com".into(),
            ipv4: "203.0.113.5".into(),
            ..DerpNode::default()
        };
        let t = DerpTarget::from_node(&no).unwrap();
        assert_eq!(t.port, 443);
        assert!(t.https);
        assert_eq!(t.ip, Some("203.0.113.5".parse().unwrap()));
        assert_eq!(t.host, "derp1.exemplo.com");
        no.cert_name = "outro.exemplo.com".into();
        no.derp_port = 8443;
        no.ipv4 = "none".into();
        no.insecure_for_tests = true;
        let t = DerpTarget::from_node(&no).unwrap();
        assert_eq!((t.port, t.ip, t.https), (8443, None, false));
        assert_eq!(t.host, "outro.exemplo.com");
        no.stun_only = true;
        assert!(DerpTarget::from_node(&no).is_none());
    }
}
