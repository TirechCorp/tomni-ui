//! O mapa em fluxo: o `POST /machine/map` que não termina e o laço que o mantém de pé.
//!
//! A resposta é uma sequência de quadros `[tamanho: u32 little-endian][JSON]`, cada um
//! comprimido com `zstd` quando o cliente pediu `Compress: "zstd"` (o crate reconhece o `zstd`
//! pela assinatura do formato, então um servidor que ignora o pedido também serve). O primeiro
//! quadro traz o mapa inteiro; os seguintes trazem só o que mudou; entre eles chegam sinais de
//! vida (`KeepAlive`).
//!
//! O [`MapStream`] lê os quadros. A [`spawn`] roda o laço completo numa tarefa: abre o fluxo,
//! aplica cada resposta no [`NetMap`], avisa quem usa por [`MapEvent`], reconecta com espera
//! crescente quando o fluxo cai e manda ao controle os endpoints novos sem derrubar o fluxo.

use std::io::Read;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use super::{Backoff, Control, ControlError, Requester};
use crate::key::{DiscoPublic, NodePublic};
use crate::netmap::{Change, NetMap};
use crate::tailcfg::{CAPABILITY_VERSION, EndpointType, Hostinfo, MapRequest, MapResponse};

/// O maior quadro, comprimido, que o crate aceita.
pub const MAIOR_QUADRO: usize = 16 * 1024 * 1024;
/// O maior JSON, já descomprimido, que o crate aceita.
pub const MAIOR_JSON: usize = 64 * 1024 * 1024;
/// A assinatura de um quadro `zstd`.
const MAGIA_ZSTD: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// O tempo para o servidor responder o cabeçalho do fluxo.
const TEMPO_ABRIR: Duration = Duration::from_secs(60);
/// O tempo do pedido de atualização dos endpoints.
const TEMPO_ATUALIZAR: Duration = Duration::from_secs(30);
/// Sem nenhum quadro (nem sinal de vida) por este tempo, o fluxo é dado como morto. O
/// servidor manda um sinal de vida a cada minuto.
pub const SILENCIO_PADRAO: Duration = Duration::from_secs(180);

/// O fluxo de respostas de um pedido de mapa, quadro a quadro.
pub struct MapStream {
    corpo: h2::RecvStream,
    buf: Vec<u8>,
}

impl MapStream {
    pub(super) fn new(corpo: h2::RecvStream) -> Self {
        MapStream {
            corpo,
            buf: Vec::new(),
        }
    }

    /// A próxima resposta do servidor; `None` quando ele encerra o fluxo.
    ///
    /// Pode ser cancelada e chamada de novo sem perder dados (o que já chegou fica guardado).
    pub async fn next(&mut self) -> Result<Option<MapResponse>, ControlError> {
        loop {
            if let Some(r) = self.extrair()? {
                return Ok(Some(r));
            }
            match self.corpo.data().await {
                Some(Ok(pedaco)) => {
                    let _ = self.corpo.flow_control().release_capacity(pedaco.len());
                    self.buf.extend_from_slice(&pedaco);
                }
                Some(Err(e)) => return Err(ControlError::Http2(e.to_string())),
                None if self.buf.is_empty() => return Ok(None),
                None => {
                    return Err(ControlError::Frame(
                        "o fluxo acabou no meio de um quadro".into(),
                    ));
                }
            }
        }
    }

    /// Tira do buffer o próximo quadro inteiro, se já chegou.
    fn extrair(&mut self) -> Result<Option<MapResponse>, ControlError> {
        extrair_de(&mut self.buf)
    }
}

/// Tira de `buf` o próximo quadro inteiro (`[tamanho LE][conteúdo]`), se já chegou. Quadros
/// vazios são ignorados.
fn extrair_de(buf: &mut Vec<u8>) -> Result<Option<MapResponse>, ControlError> {
    loop {
        let Some(cab) = buf.first_chunk::<4>() else {
            return Ok(None);
        };
        let tamanho = u32::from_le_bytes(*cab) as usize;
        if tamanho > MAIOR_QUADRO {
            return Err(ControlError::Frame(format!(
                "um quadro de {tamanho} bytes passa do limite de {MAIOR_QUADRO}"
            )));
        }
        if buf.len() < 4 + tamanho {
            return Ok(None);
        }
        let quadro: Vec<u8> = buf.drain(..4 + tamanho).skip(4).collect();
        if quadro.is_empty() {
            continue;
        }
        return decode_frame(&quadro).map(Some);
    }
}

/// Lê o conteúdo de um quadro (sem o tamanho): JSON puro ou comprimido com `zstd`.
pub fn decode_frame(quadro: &[u8]) -> Result<MapResponse, ControlError> {
    let json: std::borrow::Cow<'_, [u8]> = if quadro.starts_with(&MAGIA_ZSTD) {
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(quadro)
            .map_err(|e| ControlError::Frame(format!("zstd: {e}")))?;
        let mut saida = Vec::new();
        // Um byte a mais que o limite para distinguir "no limite" de "passou".
        (&mut decoder)
            .take(MAIOR_JSON as u64 + 1)
            .read_to_end(&mut saida)
            .map_err(|e| ControlError::Frame(format!("zstd: {e}")))?;
        if saida.len() > MAIOR_JSON {
            return Err(ControlError::Frame(format!(
                "o mapa descomprimido passa do limite de {MAIOR_JSON} bytes"
            )));
        }
        saida.into()
    } else {
        quadro.into()
    };
    serde_json::from_slice(&json).map_err(|e| ControlError::Json(e.to_string()))
}

/// Monta um quadro do mapa (para o controle falso e os testes): o tamanho em 4 bytes
/// little-endian e o JSON, comprimido ou não.
#[cfg(any(test, feature = "test-util"))]
pub fn encode_frame(json: &[u8], zstd: bool) -> Vec<u8> {
    let carga = if zstd {
        ruzstd::encoding::compress_to_vec(json, ruzstd::encoding::CompressionLevel::Fastest)
    } else {
        json.to_vec()
    };
    let mut quadro = (carga.len() as u32).to_le_bytes().to_vec();
    quadro.extend_from_slice(&carga);
    quadro
}

/// O que o cliente conta ao controle e que muda com o tempo: os endpoints e o `Hostinfo`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapAnnounce {
    /// Onde alcançar esta máquina, e de onde veio cada endereço.
    pub endpoints: Vec<(SocketAddr, EndpointType)>,
    /// O que a máquina conta de si (inclui `ShieldsUp`, que o app deixa ligado, e nenhuma
    /// sub-rede em `RoutableIPs`).
    pub hostinfo: Hostinfo,
}

/// O que o laço do mapa precisa saber.
#[derive(Debug, Clone)]
pub struct MapConfig {
    /// A chave de nó (já registrada).
    pub node_key: NodePublic,
    /// A chave de disco.
    pub disco_key: DiscoPublic,
    /// O que anunciar de começo.
    pub announce: MapAnnounce,
    /// Se as sub-redes anunciadas por outras máquinas entram nas rotas.
    pub accept_routes: bool,
    /// Se pede o mapa comprimido com `zstd`.
    pub compress: bool,
    /// Quanto tempo sem nenhum quadro até dar o fluxo como morto.
    pub silence: Duration,
}

impl MapConfig {
    /// A configuração para as chaves dadas: `zstd` ligado, sub-redes aceitas.
    pub fn new(node_key: NodePublic, disco_key: DiscoPublic) -> Self {
        MapConfig {
            node_key,
            disco_key,
            announce: MapAnnounce::default(),
            accept_routes: true,
            compress: true,
            silence: SILENCIO_PADRAO,
        }
    }
}

/// O que o laço do mapa avisa.
#[derive(Debug, Clone)]
pub enum MapEvent {
    /// O fluxo abriu (o servidor aceitou o pedido).
    Connected,
    /// Chegou uma resposta que mexeu no mapa. A primeira de cada fluxo vem sempre, mesmo
    /// quando nada mudou, para quem usa saber que o mapa inteiro chegou.
    Update {
        /// O mapa depois da mudança.
        netmap: Arc<NetMap>,
        /// O que mudou.
        change: Change,
        /// Se é a primeira resposta do fluxo (o mapa inteiro).
        first: bool,
    },
    /// O fluxo caiu; o laço espera um pouco e tenta de novo sozinho.
    Disconnected(Arc<ControlError>),
    /// O servidor recusou de um jeito que repetir não resolve (por exemplo, não conhece mais a
    /// chave de nó). O laço parou.
    Failed(Arc<ControlError>),
}

/// O laço do mapa em andamento.
pub struct MapHandle {
    eventos: mpsc::Receiver<MapEvent>,
    anuncio: watch::Sender<MapAnnounce>,
    parar: Option<oneshot::Sender<()>>,
    tarefa: JoinHandle<Control>,
}

impl MapHandle {
    /// O próximo aviso; `None` quando o laço terminou.
    pub async fn recv(&mut self) -> Option<MapEvent> {
        self.eventos.recv().await
    }

    /// Conta ao controle os endpoints (e o `Hostinfo`) novos. Se nada mudou, não faz nada.
    pub fn announce(&self, novo: MapAnnounce) {
        self.anuncio.send_if_modified(|atual| {
            if *atual == novo {
                false
            } else {
                *atual = novo;
                true
            }
        });
    }

    /// Para o laço e devolve o `Control` (com a conexão, se ainda de pé).
    pub async fn stop(mut self) -> Option<Control> {
        if let Some(p) = self.parar.take() {
            let _ = p.send(());
        }
        self.tarefa.await.ok()
    }
}

/// Sobe o laço do mapa numa tarefa do runtime atual.
pub fn spawn(control: Control, config: MapConfig) -> MapHandle {
    let (tx, rx) = mpsc::channel(32);
    let (anuncio_tx, anuncio_rx) = watch::channel(config.announce.clone());
    let (parar_tx, parar_rx) = oneshot::channel();
    let tarefa = tokio::spawn(laco(control, config, tx, anuncio_rx, parar_rx));
    MapHandle {
        eventos: rx,
        anuncio: anuncio_tx,
        parar: Some(parar_tx),
        tarefa,
    }
}

fn montar(config: &MapConfig, anuncio: &MapAnnounce, fluxo: bool) -> MapRequest {
    MapRequest {
        version: CAPABILITY_VERSION,
        compress: if fluxo && config.compress {
            "zstd".into()
        } else {
            String::new()
        },
        keep_alive: fluxo,
        node_key: config.node_key,
        disco_key: config.disco_key,
        stream: fluxo,
        hostinfo: Some(anuncio.hostinfo.clone()),
        endpoints: anuncio.endpoints.iter().map(|(a, _)| *a).collect(),
        endpoint_types: anuncio.endpoints.iter().map(|(_, t)| *t).collect(),
        // A atualização de endpoints não quer a lista de pares de volta.
        omit_peers: !fluxo,
        read_only: false,
    }
}

async fn abrir(
    control: &mut Control,
    pedido: &MapRequest,
) -> Result<(MapStream, Requester), ControlError> {
    let mut ultimo = None;
    // Uma conexão velha pode ter caído sem ninguém ver: a primeira falha de HTTP/2 refaz a
    // conexão e tenta uma vez mais, sem esperar.
    for _ in 0..2 {
        let pedidos = control.connection().await?.requester();
        match pedidos.open_map(pedido, TEMPO_ABRIR).await {
            Ok(fluxo) => return Ok((fluxo, pedidos)),
            Err(e) => {
                control.disconnect();
                if !matches!(e, ControlError::Http2(_)) {
                    return Err(e);
                }
                ultimo = Some(e);
            }
        }
    }
    Err(ultimo.expect("duas tentativas falharam"))
}

/// Espera `tempo`; devolve `true` se mandaram parar antes.
async fn esperar(parar: &mut oneshot::Receiver<()>, tempo: Duration) -> bool {
    tokio::select! {
        _ = parar => true,
        _ = tokio::time::sleep(tempo) => false,
    }
}

async fn laco(
    mut control: Control,
    config: MapConfig,
    tx: mpsc::Sender<MapEvent>,
    mut anuncio: watch::Receiver<MapAnnounce>,
    mut parar: oneshot::Receiver<()>,
) -> Control {
    let mut backoff: Backoff = control.config.backoff.clone();
    let mut netmap = NetMap::new(config.accept_routes);
    'fora: loop {
        let atual = anuncio.borrow_and_update().clone();
        let pedido = montar(&config, &atual, true);
        let aberto = tokio::select! {
            r = abrir(&mut control, &pedido) => r,
            _ = &mut parar => break 'fora,
        };
        let (mut fluxo, pedidos) = match aberto {
            Ok(x) => x,
            Err(e) => {
                let transitorio = e.is_transient();
                let e = Arc::new(e);
                if !transitorio {
                    let _ = tx.send(MapEvent::Failed(e)).await;
                    break 'fora;
                }
                if tx.send(MapEvent::Disconnected(e)).await.is_err() {
                    break 'fora;
                }
                if esperar(&mut parar, backoff.next_delay()).await {
                    break 'fora;
                }
                continue 'fora;
            }
        };
        if tx.send(MapEvent::Connected).await.is_err() {
            break 'fora;
        }
        let mut primeira = true;
        let silencio = tokio::time::sleep(config.silence);
        tokio::pin!(silencio);
        let erro = loop {
            tokio::select! {
                _ = &mut parar => break 'fora,
                _ = &mut silencio => break ControlError::Silent,
                mudou = anuncio.changed() => {
                    if mudou.is_err() {
                        break 'fora;
                    }
                    let novo = anuncio.borrow_and_update().clone();
                    let pedido = montar(&config, &novo, false);
                    let pedidos = pedidos.clone();
                    // Em outra tarefa, para não parar a leitura do fluxo. Se falhar, a
                    // próxima mudança de endpoints conta de novo.
                    tokio::spawn(async move {
                        let _ = pedidos
                            .post_ignored("/machine/map", &pedido, TEMPO_ATUALIZAR)
                            .await;
                    });
                }
                quadro = fluxo.next() => match quadro {
                    Ok(Some(resposta)) => {
                        silencio
                            .as_mut()
                            .reset(tokio::time::Instant::now() + config.silence);
                        backoff.reset();
                        if resposta.keep_alive {
                            continue;
                        }
                        let change = if primeira {
                            netmap.apply_snapshot(resposta)
                        } else {
                            netmap.apply(resposta)
                        };
                        if !primeira && change.is_empty() {
                            continue;
                        }
                        let evento = MapEvent::Update {
                            netmap: Arc::new(netmap.clone()),
                            change,
                            first: primeira,
                        };
                        primeira = false;
                        if tx.send(evento).await.is_err() {
                            break 'fora;
                        }
                    }
                    Ok(None) => break ControlError::Http2(
                        "o servidor encerrou o fluxo do mapa".into(),
                    ),
                    Err(e) => break e,
                },
            }
        };
        // O fluxo caiu: a conexão toda é refeita (pode ser ela a morta).
        control.disconnect();
        let transitorio = erro.is_transient();
        let erro = Arc::new(erro);
        if !transitorio {
            let _ = tx.send(MapEvent::Failed(erro)).await;
            break 'fora;
        }
        if tx.send(MapEvent::Disconnected(erro)).await.is_err() {
            break 'fora;
        }
        if esperar(&mut parar, backoff.next_delay()).await {
            break 'fora;
        }
    }
    control
}

#[cfg(test)]
mod testes {
    use super::*;

    const ZSTD_REAL: &[u8] = include_bytes!("../../tests/amostras/map_completo.json.zst");
    const JSON_REAL: &[u8] = include_bytes!("../../tests/amostras/map_completo.json");
    const ZSTD_500: &[u8] = include_bytes!("../../tests/amostras/map_500_pares.json.zst");

    #[test]
    fn le_um_quadro_de_json_puro_com_tamanho_little_endian() {
        // `{"KeepAlive":true}` tem 18 bytes: 0x12 0 0 0.
        let mut buf = vec![0x12, 0, 0, 0];
        buf.extend_from_slice(br#"{"KeepAlive":true}"#);
        let r = extrair_de(&mut buf).unwrap().unwrap();
        assert!(r.keep_alive);
        assert!(buf.is_empty());
    }

    #[test]
    fn espera_o_quadro_chegar_inteiro() {
        let inteiro = encode_frame(br#"{"KeepAlive":true}"#, false);
        let mut buf = Vec::new();
        for (i, b) in inteiro.iter().enumerate() {
            assert!(extrair_de(&mut buf).unwrap().is_none(), "no byte {i}");
            buf.push(*b);
        }
        assert!(extrair_de(&mut buf).unwrap().unwrap().keep_alive);
    }

    #[test]
    fn separa_quadros_colados_e_ignora_os_vazios() {
        let mut buf = encode_frame(br#"{"KeepAlive":true}"#, false);
        buf.extend_from_slice(&[0, 0, 0, 0]);
        buf.extend(encode_frame(br#"{"Domain":"a.ts.net"}"#, true));
        assert!(extrair_de(&mut buf).unwrap().unwrap().keep_alive);
        assert_eq!(extrair_de(&mut buf).unwrap().unwrap().domain, "a.ts.net");
        assert!(extrair_de(&mut buf).unwrap().is_none());
    }

    #[test]
    fn descomprime_um_quadro_zstd_feito_pelo_programa_de_referencia() {
        // `map_completo.json.zst` saiu do `zstd -3` da linha de comando.
        let da_amostra: MapResponse = serde_json::from_slice(JSON_REAL).unwrap();
        let lido = decode_frame(ZSTD_REAL).unwrap();
        assert_eq!(lido, da_amostra);
        assert!(!lido.peers.is_empty());
    }

    #[test]
    fn descomprime_um_mapa_grande_de_quinhentos_pares() {
        // Janela de 8 MiB e nível 19: o formato que um controle grande usa.
        let r = decode_frame(ZSTD_500).unwrap();
        assert_eq!(r.peers.len(), 500);
        assert_eq!(r.domain, "exemplo.ts.net");
    }

    #[test]
    fn comprime_e_descomprime_sem_perder_nada() {
        let json = serde_json::to_vec(&decode_frame(ZSTD_REAL).unwrap()).unwrap();
        for zstd in [false, true] {
            let mut buf = encode_frame(&json, zstd);
            let r = extrair_de(&mut buf).unwrap().unwrap();
            assert_eq!(r, decode_frame(ZSTD_REAL).unwrap(), "zstd = {zstd}");
        }
    }

    #[test]
    fn quadro_zstd_cortado_e_erro_de_quadro() {
        let cortado = &ZSTD_REAL[..ZSTD_REAL.len() / 2];
        let e = decode_frame(cortado).unwrap_err();
        assert!(matches!(e, ControlError::Frame(_)), "{e:?}");
        assert!(e.is_transient());
    }

    #[test]
    fn json_invalido_e_erro_de_json() {
        let e = decode_frame(b"{nao e json").unwrap_err();
        assert!(matches!(e, ControlError::Json(_)), "{e:?}");
    }

    #[test]
    fn tamanho_absurdo_e_recusado_antes_de_esperar_os_bytes() {
        let mut buf = (MAIOR_QUADRO as u32 + 1).to_le_bytes().to_vec();
        let e = extrair_de(&mut buf).unwrap_err();
        assert!(matches!(e, ControlError::Frame(_)), "{e:?}");
    }

    #[test]
    fn o_pedido_de_fluxo_pede_zstd_e_o_de_atualizacao_nao() {
        let nk = crate::key::NodeKey::generate().public();
        let dk = crate::key::DiscoKey::generate().public();
        let mut cfg = MapConfig::new(nk, dk);
        let anuncio = MapAnnounce::default();
        let fluxo = montar(&cfg, &anuncio, true);
        assert_eq!(fluxo.compress, "zstd");
        assert!(fluxo.stream && fluxo.keep_alive && !fluxo.omit_peers);
        let atualiza = montar(&cfg, &anuncio, false);
        assert_eq!(atualiza.compress, "");
        assert!(!atualiza.stream && atualiza.omit_peers);
        cfg.compress = false;
        assert_eq!(montar(&cfg, &anuncio, true).compress, "");
    }
}
