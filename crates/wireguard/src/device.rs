//! O dispositivo WireGuard: a identidade, os pares, o roteamento por chave e o endpoint de cada
//! par, tudo sem soquete e sem relógio próprio.
//!
//! O motor que liga isto a um `Underlay` (do `tomni-ui-netstack`) só precisa de quatro gestos:
//! [`Device::send`] para os pacotes IP que saem, [`Device::receive`] para os datagramas que
//! chegam, [`Device::poll`] para esvaziar o que há para mandar (e rodar os relógios) e
//! [`Device::next_deadline`] para saber até quando dormir.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use crate::handshake::{Identity, TIPO_INICIACAO};
use crate::indices::Indices;
use crate::key::{KeyError, PrivateKey, PublicKey};
use crate::peer::{Output, Peer, PeerConfig, PeerError, Processado};
use crate::responder::{Inbound, Responder};
use crate::router::{Cidr, Router};

/// Um erro ao mexer nos pares do dispositivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceError {
    /// Já existe um par com esta chave pública.
    PeerExists(PublicKey),
    /// A chave pública do par é a deste mesmo dispositivo.
    SelfPeer,
    /// Um problema com as chaves.
    Key(KeyError),
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::PeerExists(k) => write!(f, "o par {} já existe", k.short()),
            DeviceError::SelfPeer => write!(f, "a chave do par é a deste mesmo dispositivo"),
            DeviceError::Key(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DeviceError {}

impl From<KeyError> for DeviceError {
    fn from(e: KeyError) -> Self {
        DeviceError::Key(e)
    }
}

/// Por que um pacote IP não pôde ser enfileirado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// Não é um pacote IPv4 ou IPv6 inteiro.
    NotIp,
    /// Nenhum par tem uma faixa que cubra o destino.
    NoRoute(IpAddr),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::NotIp => write!(f, "o pacote não é IPv4 nem IPv6"),
            SendError::NoRoute(ip) => write!(f, "nenhum par aceita o destino {ip}"),
        }
    }
}

impl std::error::Error for SendError {}

/// O que o dispositivo produziu.
#[derive(Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// Nada (mais) a fazer.
    Nothing,
    /// Mandar este datagrama UDP a `to`.
    Transmit {
        /// O endereço do par.
        to: SocketAddr,
        /// Os bytes do datagrama.
        data: &'a [u8],
    },
    /// Entregar este pacote IP à pilha de rede; `from` é o par que o mandou.
    Ip {
        /// O par de que o pacote veio.
        from: PublicKey,
        /// O pacote IP.
        data: &'a [u8],
    },
    /// Um par precisa de atenção (por exemplo, não respondeu ao handshake).
    PeerError {
        /// O par.
        peer: PublicKey,
        /// O que houve.
        error: PeerError,
    },
}

struct Vaga {
    peer: Peer,
    endpoint: Option<SocketAddr>,
}

/// O dispositivo WireGuard.
pub struct Device {
    identity: Arc<Identity>,
    indices: Indices,
    responder: Responder,
    pares: HashMap<PublicKey, Vaga>,
    rotas: Router<PublicKey>,
    respostas: VecDeque<(SocketAddr, Vec<u8>)>,
    saida: Vec<u8>,
    de: Option<PublicKey>,
}

impl Device {
    /// Cria o dispositivo com a chave privada.
    pub fn new(chave: PrivateKey, now: Instant) -> Result<Self, KeyError> {
        let identity = Arc::new(Identity::new(chave));
        Ok(Device {
            responder: Responder::new(identity.clone(), now)?,
            identity,
            indices: Indices::new(),
            pares: HashMap::new(),
            rotas: Router::new(),
            respostas: VecDeque::new(),
            saida: Vec::new(),
            de: None,
        })
    }

    /// A chave pública deste dispositivo.
    pub fn public_key(&self) -> PublicKey {
        self.identity.public()
    }

    /// Liga ou desliga o controle de carga das iniciações (cookies).
    pub fn set_under_load(&mut self, sob_carga: bool) {
        self.responder.set_under_load(sob_carga);
    }

    /// Acrescenta um par com as faixas que ele pode usar (`AllowedIPs`).
    pub fn add_peer(
        &mut self,
        config: PeerConfig,
        endpoint: Option<SocketAddr>,
        allowed_ips: &[Cidr],
        now: Instant,
    ) -> Result<(), DeviceError> {
        let chave = config.public_key;
        if chave == self.identity.public() {
            return Err(DeviceError::SelfPeer);
        }
        if self.pares.contains_key(&chave) {
            return Err(DeviceError::PeerExists(chave));
        }
        let peer = Peer::new(self.identity.clone(), self.indices.clone(), config, now)?;
        for faixa in allowed_ips {
            self.rotas.insert(*faixa, chave);
        }
        self.pares.insert(chave, Vaga { peer, endpoint });
        Ok(())
    }

    /// Tira um par (e suas faixas). Devolve se ele existia.
    pub fn remove_peer(&mut self, chave: &PublicKey) -> bool {
        self.rotas.remove_value(chave);
        self.pares.remove(chave).is_some()
    }

    /// O par com esta chave.
    pub fn peer(&self, chave: &PublicKey) -> Option<&Peer> {
        self.pares.get(chave).map(|v| &v.peer)
    }

    /// O par com esta chave, para mudar.
    pub fn peer_mut(&mut self, chave: &PublicKey) -> Option<&mut Peer> {
        self.pares.get_mut(chave).map(|v| &mut v.peer)
    }

    /// As chaves dos pares.
    pub fn peers(&self) -> impl Iterator<Item = &PublicKey> {
        self.pares.keys()
    }

    /// O endpoint conhecido do par.
    pub fn endpoint(&self, chave: &PublicKey) -> Option<SocketAddr> {
        self.pares.get(chave).and_then(|v| v.endpoint)
    }

    /// Define o endpoint do par.
    pub fn set_endpoint(&mut self, chave: &PublicKey, endpoint: Option<SocketAddr>) {
        if let Some(v) = self.pares.get_mut(chave) {
            v.endpoint = endpoint;
        }
    }

    /// Que par atende a este endereço de destino.
    pub fn route(&self, ip: IpAddr) -> Option<PublicKey> {
        self.rotas.lookup(ip).copied()
    }

    /// Enfileira um pacote IP que sai: o destino escolhe o par pelas faixas. O pacote sai em
    /// [`Device::poll`].
    pub fn send(&mut self, pacote: &[u8]) -> Result<PublicKey, SendError> {
        let destino = destino_ip(pacote).ok_or(SendError::NotIp)?;
        let chave = *self
            .rotas
            .lookup(destino)
            .ok_or(SendError::NoRoute(destino))?;
        if let Some(v) = self.pares.get_mut(&chave) {
            v.peer.queue(pacote);
        }
        Ok(chave)
    }

    /// Processa um datagrama UDP que chegou de `de`. Devolve um pacote IP para a pilha, ou
    /// (como [`Device::poll`]) a próxima coisa a mandar.
    pub fn receive(&mut self, datagrama: &[u8], de: SocketAddr, now: Instant) -> Event<'_> {
        if datagrama.len() < 4 {
            return Event::Nothing;
        }
        let tipo = u32::from_le_bytes([datagrama[0], datagrama[1], datagrama[2], datagrama[3]]);
        let chave = if tipo == TIPO_INICIACAO {
            match self.responder.open(datagrama, de, now) {
                Inbound::Initiation(aberta) => {
                    let chave = aberta.peer();
                    if let Some(v) = self.pares.get_mut(&chave)
                        && v.peer.accept(aberta, now).is_ok()
                    {
                        v.endpoint = Some(de);
                        v.peer.take_authenticated();
                    }
                    None
                }
                Inbound::Reply(r) => {
                    self.respostas.push_back((de, r));
                    None
                }
                Inbound::Drop => None,
            }
        } else {
            // O índice do receptor fica em lugares diferentes conforme o tipo.
            let posicao = match tipo {
                2 if datagrama.len() >= 12 => Some(8),
                3 | 4 if datagrama.len() >= 8 => Some(4),
                _ => None,
            };
            posicao.and_then(|p| {
                let indice = u32::from_le_bytes([
                    datagrama[p],
                    datagrama[p + 1],
                    datagrama[p + 2],
                    datagrama[p + 3],
                ]);
                self.indices.lookup(indice)
            })
        };
        let mut entregue = false;
        if let Some(chave) = chave
            && let Some(v) = self.pares.get_mut(&chave)
        {
            let resultado = v.peer.processar(datagrama, now);
            if v.peer.take_authenticated() {
                v.endpoint = Some(de);
            }
            if let Processado::Ip(n) = resultado {
                let claro = v.peer.plain(n);
                // O endereço de origem tem de pertencer ao par que mandou o pacote.
                let permitido = origem_ip(claro)
                    .and_then(|ip| self.rotas.lookup(ip))
                    .is_some_and(|k| *k == chave);
                if permitido {
                    self.saida.clear();
                    self.saida.extend_from_slice(claro);
                    self.de = Some(chave);
                    entregue = true;
                }
            }
        }
        if entregue && let Some(from) = self.de.take() {
            return Event::Ip {
                from,
                data: &self.saida,
            };
        }
        self.poll(now)
    }

    /// Roda os relógios e devolve a próxima coisa a mandar (ou um aviso). Chame até vir
    /// [`Event::Nothing`], depois de qualquer chamada ao dispositivo e quando
    /// [`Device::next_deadline`] chegar.
    pub fn poll(&mut self, now: Instant) -> Event<'_> {
        if let Some((para, bytes)) = self.respostas.pop_front() {
            self.saida = bytes;
            return Event::Transmit {
                to: para,
                data: &self.saida,
            };
        }
        let mut achado: Option<Result<SocketAddr, (PublicKey, PeerError)>> = None;
        'pares: for (chave, v) in self.pares.iter_mut() {
            // `update_timers` também esvazia a saída do par; as chamadas seguintes só tiram o
            // que sobrou.
            let mut saida = v.peer.update_timers(now);
            loop {
                match saida {
                    Output::Network(dados) => {
                        // Sem endpoint conhecido não há para onde mandar: o pacote se perde,
                        // e o WireGuard recomeça o handshake no relógio dele.
                        if let Some(para) = v.endpoint {
                            self.saida.clear();
                            self.saida.extend_from_slice(dados);
                            achado = Some(Ok(para));
                            break 'pares;
                        }
                    }
                    Output::Error(error) => {
                        achado = Some(Err((*chave, error)));
                        break 'pares;
                    }
                    Output::Ip(_) | Output::Nothing => break,
                }
                saida = v.peer.poll_transmit(now);
            }
        }
        match achado {
            Some(Ok(to)) => Event::Transmit {
                to,
                data: &self.saida,
            },
            Some(Err((peer, error))) => Event::PeerError { peer, error },
            None => Event::Nothing,
        }
    }

    /// O próximo instante em que [`Device::poll`] precisa rodar por causa dos relógios.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pares
            .values()
            .filter_map(|v| v.peer.next_deadline())
            .min()
    }
}

/// O endereço de destino de um pacote IP inteiro.
pub(crate) fn destino_ip(p: &[u8]) -> Option<IpAddr> {
    match p.first()? >> 4 {
        4 if p.len() >= 20 => Some(IpAddr::V4(Ipv4Addr::new(p[16], p[17], p[18], p[19]))),
        6 if p.len() >= 40 => {
            let mut b = [0u8; 16];
            b.copy_from_slice(&p[24..40]);
            Some(IpAddr::V6(Ipv6Addr::from(b)))
        }
        _ => None,
    }
}

/// O endereço de origem de um pacote IP inteiro.
pub(crate) fn origem_ip(p: &[u8]) -> Option<IpAddr> {
    match p.first()? >> 4 {
        4 if p.len() >= 20 => Some(IpAddr::V4(Ipv4Addr::new(p[12], p[13], p[14], p[15]))),
        6 if p.len() >= 40 => {
            let mut b = [0u8; 16];
            b.copy_from_slice(&p[8..24]);
            Some(IpAddr::V6(Ipv6Addr::from(b)))
        }
        _ => None,
    }
}
