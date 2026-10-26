//! O "magicsock": decide, para cada par, por onde o pacote WireGuard vai (direto pelo UDP ou
//! pelo DERP) e cuida do descobrimento do caminho direto (disco).
//!
//! Este módulo não faz entrada nem saída: recebe os datagramas que chegaram e a hora, e devolve
//! o que mandar ([`Out`]). O motor ([`crate::engine`]) liga isso ao UDP e ao DERP.
//!
//! Como o WireGuard pensa em endereços UDP, cada par ganha um endereço **virtual** (um
//! `SocketAddr` que nunca vai à rede). O `Device` do WireGuard recebe e manda para esse
//! endereço; aqui ele vira "direto no endereço tal" ou "pelo DERP da região tal".
//!
//! Regras de segurança, para o app ficar invisível:
//! - um datagrama UDP só vira WireGuard se vem de um endereço que o par anunciou no mapa ou que
//!   respondeu a um ping de disco; o resto é descartado em silêncio;
//! - uma mensagem de disco só é respondida se a caixa abre com a chave de disco de um par do
//!   mapa.
//!
//! Portado do Tailscale (github.com/tailscale/tailscale, `wgengine/magicsock`, licença
//! BSD-3-Clause), em versão reduzida: IPv4, sem retransmissão de pings de latência e sem o
//! `peer relay`.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::disco::{self, Message, TxId};
use crate::key::{DiscoKey, DiscoPublic, NodePublic};
use crate::nacl::NaclBox;
use crate::tailcfg::DerpRegionId;

/// Quanto tempo um endereço direto vale depois do último pong.
pub const TRUST_DIRECT: Duration = Duration::from_millis(6500);
/// Quanto tempo se espera o pong de um ping.
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);
/// O intervalo mínimo entre duas rodadas de descobrimento para o mesmo par.
pub const DISCO_INTERVAL: Duration = Duration::from_secs(5);
/// O intervalo dos pings que mantêm o caminho direto vivo.
pub const HEARTBEAT: Duration = Duration::from_secs(3);
/// Depois deste tempo sem tráfego, o par deixa de ser mantido (nem heartbeat, nem descobrimento).
pub const SESSION_ACTIVE: Duration = Duration::from_secs(45);
/// O mais que se guarda de candidatos por par.
const MAX_CANDIDATES: usize = 32;
/// O mais que se guarda de pings esperando pong.
const MAX_PENDING: usize = 256;

/// Por onde os pacotes de um par estão indo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// Ainda não há caminho: nem direto nem DERP (o par não tem região de casa).
    None,
    /// Pelo relé da região.
    Derp(DerpRegionId),
    /// Direto, neste endereço.
    Direct(SocketAddr),
}

/// O que mandar para a rede.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// Um datagrama UDP.
    Udp {
        /// O destino.
        to: SocketAddr,
        /// Os bytes.
        data: Vec<u8>,
    },
    /// Um pacote pelo DERP.
    Derp {
        /// A região por onde mandar.
        region: DerpRegionId,
        /// O nó de destino.
        to: NodePublic,
        /// Os bytes.
        data: Vec<u8>,
    },
}

/// O que fazer com o que chegou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recv {
    /// É WireGuard do par com este endereço virtual: entregue ao `Device`.
    Wireguard(SocketAddr),
    /// Era disco (ou outra coisa nossa) e já foi tratado.
    Handled,
    /// Não é de ninguém conhecido: descartado em silêncio.
    Drop,
}

/// O que o mapa diz de um par.
#[derive(Debug, Clone)]
pub struct PeerSpec {
    /// A chave de nó.
    pub key: NodePublic,
    /// A chave de disco (zerada = sem descobrimento, só DERP).
    pub disco: DiscoPublic,
    /// Os endpoints que o par anunciou.
    pub endpoints: Vec<SocketAddr>,
    /// A região do DERP de casa do par (0 = nenhuma).
    pub home_derp: DerpRegionId,
}

struct Candidato {
    addr: SocketAddr,
    ultimo_ping: Option<Instant>,
}

struct Par {
    disco: DiscoPublic,
    caixa: Option<Arc<NaclBox>>,
    virtual_addr: SocketAddr,
    casa: DerpRegionId,
    ultima_regiao: DerpRegionId,
    do_mapa: Vec<Candidato>,
    aprendidos: Vec<Candidato>,
    direto: Option<SocketAddr>,
    confia_ate: Option<Instant>,
    latencia: Option<Duration>,
    ultima_rodada: Option<Instant>,
    ultimo_envio: Option<Instant>,
    ultimo_heartbeat: Option<Instant>,
}

impl Par {
    fn confiavel(&self, agora: Instant) -> Option<SocketAddr> {
        match (self.direto, self.confia_ate) {
            (Some(a), Some(ate)) if agora < ate => Some(a),
            _ => None,
        }
    }

    fn regiao(&self) -> DerpRegionId {
        if self.casa != 0 {
            self.casa
        } else {
            self.ultima_regiao
        }
    }

    fn ativo(&self, agora: Instant) -> bool {
        self.ultimo_envio
            .is_some_and(|t| agora.duration_since(t) < SESSION_ACTIVE)
    }

    fn candidato(&mut self, addr: SocketAddr) -> Option<&mut Candidato> {
        self.do_mapa
            .iter_mut()
            .chain(self.aprendidos.iter_mut())
            .find(|c| c.addr == addr)
    }

    fn todos(&self) -> impl Iterator<Item = &Candidato> {
        self.do_mapa.iter().chain(self.aprendidos.iter())
    }
}

struct Pendente {
    par: NodePublic,
    /// `None` quando o ping foi pelo DERP.
    para: Option<SocketAddr>,
    enviado: Instant,
}

/// O estado dos caminhos de todos os pares.
pub struct Magic {
    disco: DiscoKey,
    disco_publica: DiscoPublic,
    no: NodePublic,
    udp: bool,
    pares: HashMap<NodePublic, Par>,
    por_disco: HashMap<DiscoPublic, NodePublic>,
    por_endereco: HashMap<SocketAddr, NodePublic>,
    por_virtual: HashMap<SocketAddr, NodePublic>,
    pendentes: HashMap<TxId, Pendente>,
    nossos: Vec<SocketAddr>,
    proximo_virtual: u32,
}

/// O endereço virtual de um par: `127.66.x.y:41641`, nunca vai à rede.
fn endereco_virtual(indice: u32) -> SocketAddr {
    let b = indice.to_be_bytes();
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 66, b[2], b[3])), 41641)
}

fn aleatorio<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("o sistema não deu números aleatórios");
    b
}

/// O endereço que vai no pong de um ping que veio pelo DERP (o mesmo truque do Tailscale).
fn endereco_derp(regiao: DerpRegionId) -> SocketAddr {
    SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(127, 3, 3, 40)),
        u16::try_from(regiao).unwrap_or(0),
    )
}

impl Magic {
    /// Sem nenhum par. `udp` falso = só DERP (nenhum pacote UDP sai nem entra).
    pub fn new(no: NodePublic, disco: DiscoKey, udp: bool) -> Self {
        Magic {
            disco_publica: disco.public(),
            disco,
            no,
            udp,
            pares: HashMap::new(),
            por_disco: HashMap::new(),
            por_endereco: HashMap::new(),
            por_virtual: HashMap::new(),
            pendentes: HashMap::new(),
            nossos: Vec::new(),
            proximo_virtual: 1,
        }
    }

    /// A nossa chave de disco pública (vai no mapa).
    pub fn disco_public(&self) -> DiscoPublic {
        self.disco_publica
    }

    /// Define os nossos endpoints (vão no `CallMeMaybe`).
    pub fn set_our_endpoints(&mut self, eps: Vec<SocketAddr>) {
        self.nossos = eps.iter().filter_map(disco::usable_v4).collect();
    }

    /// Acrescenta ou atualiza um par. Devolve o endereço virtual dele.
    pub fn set_peer(&mut self, spec: PeerSpec) -> SocketAddr {
        let usaveis: Vec<SocketAddr> = spec
            .endpoints
            .iter()
            .filter_map(disco::usable_v4)
            .take(MAX_CANDIDATES)
            .collect();
        if let Some(p) = self.pares.get_mut(&spec.key) {
            // Mudou a chave de disco: o caminho anterior não vale mais.
            if p.disco != spec.disco {
                self.por_disco.remove(&p.disco);
                p.disco = spec.disco;
                p.caixa = caixa(&self.disco, &spec.disco);
                p.direto = None;
                p.confia_ate = None;
                p.latencia = None;
                if !spec.disco.is_zero() {
                    self.por_disco.insert(spec.disco, spec.key);
                }
            }
            p.casa = spec.home_derp;
            // Endereços que o mapa não lista mais saem da tabela de origem.
            for c in &p.do_mapa {
                if !usaveis.contains(&c.addr) {
                    self.por_endereco.remove(&c.addr);
                }
            }
            let antigos = std::mem::take(&mut p.do_mapa);
            p.do_mapa = usaveis
                .iter()
                .map(|a| Candidato {
                    addr: *a,
                    ultimo_ping: antigos
                        .iter()
                        .find(|c| c.addr == *a)
                        .and_then(|c| c.ultimo_ping),
                })
                .collect();
            for a in &usaveis {
                self.por_endereco.insert(*a, spec.key);
            }
            return p.virtual_addr;
        }
        let virtual_addr = endereco_virtual(self.proximo_virtual);
        self.proximo_virtual = self.proximo_virtual.wrapping_add(1).max(1);
        let par = Par {
            disco: spec.disco,
            caixa: caixa(&self.disco, &spec.disco),
            virtual_addr,
            casa: spec.home_derp,
            ultima_regiao: 0,
            do_mapa: usaveis
                .iter()
                .map(|a| Candidato {
                    addr: *a,
                    ultimo_ping: None,
                })
                .collect(),
            aprendidos: Vec::new(),
            direto: None,
            confia_ate: None,
            latencia: None,
            ultima_rodada: None,
            ultimo_envio: None,
            ultimo_heartbeat: None,
        };
        if !spec.disco.is_zero() {
            self.por_disco.insert(spec.disco, spec.key);
        }
        for a in &usaveis {
            self.por_endereco.insert(*a, spec.key);
        }
        self.por_virtual.insert(virtual_addr, spec.key);
        self.pares.insert(spec.key, par);
        virtual_addr
    }

    /// Tira um par.
    pub fn remove_peer(&mut self, chave: &NodePublic) {
        if let Some(p) = self.pares.remove(chave) {
            self.por_disco.remove(&p.disco);
            self.por_virtual.remove(&p.virtual_addr);
        }
        self.por_endereco.retain(|_, k| k != chave);
        self.pendentes.retain(|_, p| p.par != *chave);
    }

    /// O endereço virtual de um par.
    pub fn virtual_addr(&self, chave: &NodePublic) -> Option<SocketAddr> {
        self.pares.get(chave).map(|p| p.virtual_addr)
    }

    /// Qual par tem este endereço virtual.
    pub fn peer_for_virtual(&self, virtual_addr: &SocketAddr) -> Option<NodePublic> {
        self.por_virtual.get(virtual_addr).copied()
    }

    /// Por onde os pacotes do par estão indo agora.
    pub fn path(&self, chave: &NodePublic, agora: Instant) -> Path {
        let Some(p) = self.pares.get(chave) else {
            return Path::None;
        };
        if self.udp
            && let Some(a) = p.confiavel(agora)
        {
            return Path::Direct(a);
        }
        match p.regiao() {
            0 => Path::None,
            r => Path::Derp(r),
        }
    }

    /// A latência do caminho direto, quando há.
    pub fn latency(&self, chave: &NodePublic) -> Option<Duration> {
        self.pares.get(chave).and_then(|p| p.latencia)
    }

    /// O `Device` quer mandar `dados` ao par de endereço virtual `virtual_addr`: escolhe o
    /// caminho e, se o direto não vale, começa o descobrimento.
    pub fn route(
        &mut self,
        virtual_addr: SocketAddr,
        dados: Vec<u8>,
        agora: Instant,
        saida: &mut Vec<Out>,
    ) {
        let Some(chave) = self.por_virtual.get(&virtual_addr).copied() else {
            return;
        };
        let Some(p) = self.pares.get_mut(&chave) else {
            return;
        };
        p.ultimo_envio = Some(agora);
        let direto = if self.udp { p.confiavel(agora) } else { None };
        match direto {
            Some(a) => saida.push(Out::Udp { to: a, data: dados }),
            None => {
                let r = p.regiao();
                if r != 0 {
                    saida.push(Out::Derp {
                        region: r,
                        to: chave,
                        data: dados,
                    });
                }
                self.descobrir(chave, agora, false, saida);
            }
        }
    }

    /// Chegou um datagrama UDP.
    pub fn on_udp(
        &mut self,
        de: SocketAddr,
        dados: &[u8],
        agora: Instant,
        saida: &mut Vec<Out>,
    ) -> Recv {
        if !self.udp {
            return Recv::Drop;
        }
        if disco::looks_like_disco(dados) {
            return self.disco_chegou(dados, Some(de), 0, None, agora, saida);
        }
        // WireGuard: só de um endereço que o par anunciou ou que respondeu a um ping.
        let Some(chave) = self.por_endereco.get(&de).copied() else {
            return Recv::Drop;
        };
        let Some(p) = self.pares.get_mut(&chave) else {
            return Recv::Drop;
        };
        if p.direto == Some(de) && p.confia_ate.is_some() {
            // Tráfego de verdade vindo do caminho direto o mantém vivo.
            p.confia_ate = Some(agora + TRUST_DIRECT);
        }
        Recv::Wireguard(p.virtual_addr)
    }

    /// Chegou um pacote pelo DERP da região `regiao`, mandado por `de`.
    pub fn on_derp(
        &mut self,
        regiao: DerpRegionId,
        de: NodePublic,
        dados: &[u8],
        agora: Instant,
        saida: &mut Vec<Out>,
    ) -> Recv {
        if disco::looks_like_disco(dados) {
            return self.disco_chegou(dados, None, regiao, Some(de), agora, saida);
        }
        let Some(p) = self.pares.get_mut(&de) else {
            return Recv::Drop;
        };
        p.ultima_regiao = regiao;
        Recv::Wireguard(p.virtual_addr)
    }

    fn disco_chegou(
        &mut self,
        dados: &[u8],
        udp_de: Option<SocketAddr>,
        regiao: DerpRegionId,
        derp_de: Option<NodePublic>,
        agora: Instant,
        saida: &mut Vec<Out>,
    ) -> Recv {
        let Some(remetente) = disco::sender(dados) else {
            return Recv::Drop;
        };
        let Some(chave) = self.por_disco.get(&remetente).copied() else {
            return Recv::Drop;
        };
        // Pelo DERP, quem manda tem de ser o dono da chave de disco.
        if derp_de.is_some_and(|d| d != chave) {
            return Recv::Drop;
        }
        let Some(msg) = self
            .pares
            .get(&chave)
            .and_then(|p| p.caixa.as_ref())
            .and_then(|c| disco::open(c, dados))
        else {
            return Recv::Drop;
        };
        if derp_de.is_some()
            && let Some(p) = self.pares.get_mut(&chave)
        {
            p.ultima_regiao = regiao;
        }
        match msg {
            Message::Ping { tx_id, .. } => {
                self.ping_chegou(chave, tx_id, udp_de, regiao, agora, saida);
            }
            Message::Pong { tx_id, src } => {
                self.pong_chegou(chave, tx_id, src, udp_de, agora);
            }
            Message::CallMeMaybe { endpoints } => {
                // O CallMeMaybe só vale pelo DERP (é assim que se prova que veio do par).
                if udp_de.is_none() {
                    self.chamar(chave, endpoints, agora, saida);
                }
            }
        }
        Recv::Handled
    }

    fn lacre(&self, chave: &NodePublic, msg: &Message) -> Option<Vec<u8>> {
        let p = self.pares.get(chave)?;
        let caixa = p.caixa.as_ref()?;
        Some(disco::seal(caixa, &self.disco_publica, msg))
    }

    fn ping_chegou(
        &mut self,
        chave: NodePublic,
        tx_id: TxId,
        udp_de: Option<SocketAddr>,
        regiao: DerpRegionId,
        agora: Instant,
        saida: &mut Vec<Out>,
    ) {
        match udp_de {
            Some(de) => {
                // O ping autêntico vindo de um endereço novo: é um candidato, e devolvemos o
                // ping para abrir o caminho nos dois sentidos.
                let novo = self.aprender(chave, de);
                if let Some(dados) = self.lacre(&chave, &Message::Pong { tx_id, src: de }) {
                    saida.push(Out::Udp {
                        to: de,
                        data: dados,
                    });
                }
                if novo {
                    self.pingar(chave, de, agora, saida);
                }
            }
            None => {
                if let Some(dados) = self.lacre(
                    &chave,
                    &Message::Pong {
                        tx_id,
                        src: endereco_derp(regiao),
                    },
                ) {
                    saida.push(Out::Derp {
                        region: regiao,
                        to: chave,
                        data: dados,
                    });
                }
                // Quem nos procura pelo DERP quer um caminho direto: vamos atrás dele.
                if self.udp {
                    self.descobrir(chave, agora, false, saida);
                }
            }
        }
    }

    fn pong_chegou(
        &mut self,
        chave: NodePublic,
        tx_id: TxId,
        _visto: SocketAddr,
        udp_de: Option<SocketAddr>,
        agora: Instant,
    ) {
        let Some(pend) = self.pendentes.remove(&tx_id) else {
            return;
        };
        if pend.par != chave {
            return;
        }
        // O pong tem de vir do endereço que pingamos.
        let (Some(de), Some(para)) = (udp_de, pend.para) else {
            return;
        };
        if de != para {
            return;
        }
        let latencia = agora.saturating_duration_since(pend.enviado);
        self.por_endereco.insert(de, chave);
        let Some(p) = self.pares.get_mut(&chave) else {
            return;
        };
        let confiavel = p.confiavel(agora);
        let melhor = match (confiavel, p.latencia) {
            (None, _) => true,
            (Some(a), _) if a == de => true,
            // Só troca de um caminho bom por outro bem mais rápido.
            (Some(_), Some(atual)) => latencia.mul_f64(1.1) < atual,
            (Some(_), None) => true,
        };
        if melhor {
            p.direto = Some(de);
            p.latencia = Some(latencia);
            p.confia_ate = Some(agora + TRUST_DIRECT);
        } else if p.direto == Some(de) {
            p.confia_ate = Some(agora + TRUST_DIRECT);
        }
    }

    /// Guarda `de` como candidato aprendido. Devolve se era novo.
    fn aprender(&mut self, chave: NodePublic, de: SocketAddr) -> bool {
        let Some(usavel) = disco::usable_v4(&de) else {
            return false;
        };
        let Some(p) = self.pares.get_mut(&chave) else {
            return false;
        };
        self.por_endereco.insert(usavel, chave);
        if p.candidato(usavel).is_some() {
            return false;
        }
        if p.aprendidos.len() >= MAX_CANDIDATES {
            p.aprendidos.remove(0);
        }
        p.aprendidos.push(Candidato {
            addr: usavel,
            ultimo_ping: None,
        });
        true
    }

    fn chamar(
        &mut self,
        chave: NodePublic,
        endpoints: Vec<SocketAddr>,
        agora: Instant,
        saida: &mut Vec<Out>,
    ) {
        let Some(p) = self.pares.get_mut(&chave) else {
            return;
        };
        // A lista do CallMeMaybe substitui a anterior.
        for c in &p.aprendidos {
            if !p.do_mapa.iter().any(|m| m.addr == c.addr) {
                self.por_endereco.remove(&c.addr);
            }
        }
        p.aprendidos = endpoints
            .iter()
            .filter_map(disco::usable_v4)
            .filter(|a| !p.do_mapa.iter().any(|m| m.addr == *a))
            .take(MAX_CANDIDATES)
            .map(|addr| Candidato {
                addr,
                ultimo_ping: None,
            })
            .collect();
        for c in &p.aprendidos {
            self.por_endereco.insert(c.addr, chave);
        }
        self.descobrir(chave, agora, true, saida);
    }

    fn pingar(
        &mut self,
        chave: NodePublic,
        para: SocketAddr,
        agora: Instant,
        saida: &mut Vec<Out>,
    ) {
        if self.pendentes.len() >= MAX_PENDING {
            return;
        }
        let tx_id: TxId = aleatorio();
        let msg = Message::Ping {
            tx_id,
            node_key: Some(self.no),
        };
        let Some(dados) = self.lacre(&chave, &msg) else {
            return;
        };
        self.pendentes.insert(
            tx_id,
            Pendente {
                par: chave,
                para: Some(para),
                enviado: agora,
            },
        );
        if let Some(c) = self.pares.get_mut(&chave).and_then(|p| p.candidato(para)) {
            c.ultimo_ping = Some(agora);
        }
        saida.push(Out::Udp {
            to: para,
            data: dados,
        });
    }

    /// Começa uma rodada de descobrimento: ping a todos os candidatos e o `CallMeMaybe` pelo
    /// DERP. `forcar` ignora o intervalo mínimo (o `CallMeMaybe` recebido).
    fn descobrir(&mut self, chave: NodePublic, agora: Instant, forcar: bool, saida: &mut Vec<Out>) {
        if !self.udp {
            return;
        }
        let Some(p) = self.pares.get_mut(&chave) else {
            return;
        };
        if p.disco.is_zero() || p.caixa.is_none() {
            return;
        }
        if !forcar
            && p.ultima_rodada
                .is_some_and(|t| agora.duration_since(t) < DISCO_INTERVAL)
        {
            return;
        }
        // Com o caminho direto em dia não precisa procurar outro.
        if !forcar && p.confiavel(agora).is_some() {
            return;
        }
        p.ultima_rodada = Some(agora);
        let alvos: Vec<SocketAddr> = p.todos().map(|c| c.addr).collect();
        let regiao = p.regiao();
        for a in alvos {
            self.pingar(chave, a, agora, saida);
        }
        if regiao != 0 && !self.nossos.is_empty() {
            let msg = Message::CallMeMaybe {
                endpoints: self.nossos.clone(),
            };
            if let Some(dados) = self.lacre(&chave, &msg) {
                saida.push(Out::Derp {
                    region: regiao,
                    to: chave,
                    data: dados,
                });
            }
        }
    }

    /// Roda os relógios: esquece pings sem resposta, mantém o caminho direto vivo e procura
    /// outro quando o direto caiu.
    pub fn tick(&mut self, agora: Instant, saida: &mut Vec<Out>) {
        self.pendentes
            .retain(|_, p| agora.duration_since(p.enviado) < PING_TIMEOUT);
        if !self.udp {
            return;
        }
        let chaves: Vec<NodePublic> = self.pares.keys().copied().collect();
        for chave in chaves {
            let Some(p) = self.pares.get_mut(&chave) else {
                continue;
            };
            if !p.ativo(agora) {
                continue;
            }
            match p.confiavel(agora) {
                Some(a) => {
                    if p.ultimo_heartbeat
                        .is_none_or(|t| agora.duration_since(t) >= HEARTBEAT)
                    {
                        p.ultimo_heartbeat = Some(agora);
                        self.pingar(chave, a, agora, saida);
                    }
                }
                None => self.descobrir(chave, agora, false, saida),
            }
        }
    }

    /// Se algum par está em uso (então o [`Magic::tick`] tem o que fazer).
    pub fn has_active_peers(&self, agora: Instant) -> bool {
        self.udp && self.pares.values().any(|p| p.ativo(agora))
    }
}

fn caixa(nossa: &DiscoKey, deles: &DiscoPublic) -> Option<Arc<NaclBox>> {
    if deles.is_zero() {
        return None;
    }
    Some(Arc::new(NaclBox::new(&nossa.to_bytes(), deles.as_bytes())))
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::key::NodeKey;

    struct Lado {
        magic: Magic,
        no: NodePublic,
        disco: DiscoPublic,
    }

    fn lado(udp: bool) -> Lado {
        let no = NodeKey::generate().public();
        let disco = DiscoKey::generate();
        let dp = disco.public();
        Lado {
            magic: Magic::new(no, disco, udp),
            no,
            disco: dp,
        }
    }

    fn ligar(a: &mut Lado, b: &Lado, eps: &[&str], casa: i32) -> SocketAddr {
        a.magic.set_peer(PeerSpec {
            key: b.no,
            disco: b.disco,
            endpoints: eps.iter().map(|e| e.parse().unwrap()).collect(),
            home_derp: casa,
        })
    }

    fn udp_para(saida: &[Out], para: &str) -> Vec<Vec<u8>> {
        let para: SocketAddr = para.parse().unwrap();
        saida
            .iter()
            .filter_map(|o| match o {
                Out::Udp { to, data } if *to == para => Some(data.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn sem_caminho_direto_o_pacote_vai_pelo_derp_e_o_descobrimento_comeca() {
        let agora = Instant::now();
        let mut a = lado(true);
        let b = lado(true);
        a.magic
            .set_our_endpoints(vec!["198.51.100.1:5000".parse().unwrap()]);
        let v = ligar(&mut a, &b, &["203.0.113.9:41641"], 2);
        let mut saida = Vec::new();
        a.magic.route(v, b"wg".to_vec(), agora, &mut saida);
        // O pacote, o ping ao candidato e o CallMeMaybe.
        assert!(saida.contains(&Out::Derp {
            region: 2,
            to: b.no,
            data: b"wg".to_vec()
        }));
        assert_eq!(udp_para(&saida, "203.0.113.9:41641").len(), 1);
        assert_eq!(
            saida
                .iter()
                .filter(|o| matches!(o, Out::Derp { data, .. } if data != b"wg"))
                .count(),
            1
        );
        assert_eq!(a.magic.path(&b.no, agora), Path::Derp(2));
        // Uma segunda chamada logo em seguida não repete a rodada.
        let mut saida = Vec::new();
        a.magic.route(v, b"wg2".to_vec(), agora, &mut saida);
        assert_eq!(saida.len(), 1);
    }

    #[test]
    fn o_pong_abre_o_caminho_direto_e_ele_expira() {
        let agora = Instant::now();
        let mut a = lado(true);
        let mut b = lado(true);
        let v_ab = ligar(&mut a, &b, &["203.0.113.9:41641"], 2);
        ligar(&mut b, &a, &[], 1);
        let mut saida = Vec::new();
        a.magic.route(v_ab, b"x".to_vec(), agora, &mut saida);
        let ping = udp_para(&saida, "203.0.113.9:41641").remove(0);
        // O ping chega em B, vindo do endereço público de A.
        let a_publico: SocketAddr = "192.0.2.50:7000".parse().unwrap();
        let mut resp = Vec::new();
        let r = b.magic.on_udp(a_publico, &ping, agora, &mut resp);
        assert_eq!(r, Recv::Handled);
        // B responde pong e, como o endereço é novo, pinga de volta.
        let pong = udp_para(&resp, "192.0.2.50:7000");
        assert_eq!(pong.len(), 2);
        // O pong volta para A.
        let depois = agora + Duration::from_millis(20);
        let mut fim = Vec::new();
        let b_publico: SocketAddr = "203.0.113.9:41641".parse().unwrap();
        for d in &pong {
            a.magic.on_udp(b_publico, d, depois, &mut fim);
        }
        assert_eq!(
            a.magic.path(&b.no, depois),
            Path::Direct("203.0.113.9:41641".parse().unwrap())
        );
        assert_eq!(a.magic.latency(&b.no), Some(Duration::from_millis(20)));
        // Com o direto valendo, o pacote vai por ele.
        let mut s = Vec::new();
        a.magic.route(v_ab, b"y".to_vec(), depois, &mut s);
        assert_eq!(
            s,
            vec![Out::Udp {
                to: b_publico,
                data: b"y".to_vec()
            }]
        );
        // E o WireGuard que vem desse endereço é entregue ao par.
        let mut s = Vec::new();
        assert_eq!(
            a.magic.on_udp(b_publico, b"\x04\0\0\0wg", depois, &mut s),
            Recv::Wireguard(v_ab)
        );
        // Sem pongs, a confiança acaba e volta para o DERP.
        let tarde = depois + TRUST_DIRECT + Duration::from_millis(1);
        assert_eq!(a.magic.path(&b.no, tarde), Path::Derp(2));
    }

    #[test]
    fn o_heartbeat_mantem_o_direto_e_a_queda_procura_de_novo() {
        let t0 = Instant::now();
        let mut a = lado(true);
        let mut b = lado(true);
        let v = ligar(&mut a, &b, &["203.0.113.9:41641"], 2);
        ligar(&mut b, &a, &[], 1);
        let mut s = Vec::new();
        a.magic.route(v, b"x".to_vec(), t0, &mut s);
        let ping = udp_para(&s, "203.0.113.9:41641").remove(0);
        let mut resp = Vec::new();
        b.magic
            .on_udp("192.0.2.50:7000".parse().unwrap(), &ping, t0, &mut resp);
        let pong = udp_para(&resp, "192.0.2.50:7000").remove(0);
        a.magic
            .on_udp("203.0.113.9:41641".parse().unwrap(), &pong, t0, &mut s);
        // Três segundos depois, o tick manda um ping ao endereço direto.
        let t1 = t0 + Duration::from_secs(3);
        let mut hb = Vec::new();
        a.magic.tick(t1, &mut hb);
        assert_eq!(udp_para(&hb, "203.0.113.9:41641").len(), 1);
        // Sem o pong, aos 6,5 s o direto vale menos e o tick recomeça a procurar.
        let t2 = t0 + Duration::from_secs(7);
        let mut procura = Vec::new();
        a.magic.tick(t2, &mut procura);
        assert_eq!(udp_para(&procura, "203.0.113.9:41641").len(), 1);
        assert_eq!(a.magic.path(&b.no, t2), Path::Derp(2));
    }

    #[test]
    fn o_ping_de_um_endereco_novo_vira_candidato_e_so_o_par_do_mapa_e_respondido() {
        let agora = Instant::now();
        let mut a = lado(true);
        let mut b = lado(true);
        let estranho = lado(true);
        let v = ligar(&mut a, &b, &[], 2);
        ligar(&mut b, &a, &["192.0.2.1:1"], 1);
        // B pinga A (sabendo o endereço dele pelo CallMeMaybe, simulado pelo mapa).
        let mut s = Vec::new();
        b.magic.route(
            b.magic.virtual_addr(&a.no).unwrap(),
            b"x".to_vec(),
            agora,
            &mut s,
        );
        let ping = udp_para(&s, "192.0.2.1:1").remove(0);
        let mut resp = Vec::new();
        let de: SocketAddr = "198.18.0.7:9999".parse().unwrap();
        assert_eq!(a.magic.on_udp(de, &ping, agora, &mut resp), Recv::Handled);
        assert_eq!(udp_para(&resp, "198.18.0.7:9999").len(), 2);
        // Agora o WireGuard desse endereço é aceito como sendo de B.
        assert_eq!(
            a.magic.on_udp(de, b"\x01\0\0\0", agora, &mut resp),
            Recv::Wireguard(v)
        );
        // Um estranho (chave de disco desconhecida) não recebe resposta nenhuma.
        let mut s2 = Vec::new();
        let mut c = lado(true);
        ligar(&mut c, &a, &["192.0.2.1:1"], 1);
        c.magic.route(
            c.magic.virtual_addr(&a.no).unwrap(),
            b"x".to_vec(),
            agora,
            &mut s2,
        );
        let ping_estranho = udp_para(&s2, "192.0.2.1:1").remove(0);
        let mut resp2 = Vec::new();
        assert_eq!(
            a.magic.on_udp(
                "198.18.0.8:1".parse().unwrap(),
                &ping_estranho,
                agora,
                &mut resp2
            ),
            Recv::Drop
        );
        assert!(resp2.is_empty());
        // Lixo e WireGuard de endereço desconhecido também caem em silêncio.
        assert_eq!(
            a.magic.on_udp(
                "198.18.0.9:1".parse().unwrap(),
                b"\x01\0\0\0",
                agora,
                &mut resp2
            ),
            Recv::Drop
        );
        assert!(resp2.is_empty());
        let _ = estranho;
    }

    #[test]
    fn o_call_me_maybe_pelo_derp_faz_pingar_os_endpoints() {
        let agora = Instant::now();
        let mut a = lado(true);
        let mut b = lado(true);
        ligar(&mut a, &b, &[], 2);
        ligar(&mut b, &a, &[], 1);
        b.magic
            .set_our_endpoints(vec!["198.51.100.4:4444".parse().unwrap()]);
        // B manda um tráfego e o CallMeMaybe sai pelo DERP de A.
        let mut s = Vec::new();
        b.magic.route(
            b.magic.virtual_addr(&a.no).unwrap(),
            b"x".to_vec(),
            agora,
            &mut s,
        );
        let cmm = s
            .iter()
            .find_map(|o| match o {
                Out::Derp { data, .. } if data != b"x" => Some(data.clone()),
                _ => None,
            })
            .expect("o CallMeMaybe");
        let mut resp = Vec::new();
        assert_eq!(
            a.magic.on_derp(1, b.no, &cmm, agora, &mut resp),
            Recv::Handled
        );
        // A pinga o endpoint anunciado.
        assert_eq!(udp_para(&resp, "198.51.100.4:4444").len(), 1);
        // Pelo UDP o CallMeMaybe não vale.
        let mut r2 = Vec::new();
        a.magic
            .on_udp("198.51.100.4:4444".parse().unwrap(), &cmm, agora, &mut r2);
        assert!(r2.is_empty());
    }

    #[test]
    fn so_derp_nunca_toca_o_udp() {
        let agora = Instant::now();
        let mut a = lado(false);
        let b = lado(true);
        let v = ligar(&mut a, &b, &["203.0.113.9:41641"], 2);
        let mut s = Vec::new();
        a.magic.route(v, b"x".to_vec(), agora, &mut s);
        assert_eq!(
            s,
            vec![Out::Derp {
                region: 2,
                to: b.no,
                data: b"x".to_vec()
            }]
        );
        let mut s = Vec::new();
        a.magic.tick(agora + Duration::from_secs(10), &mut s);
        assert!(s.is_empty());
        assert_eq!(
            a.magic.on_udp(
                "203.0.113.9:41641".parse().unwrap(),
                b"\x01\0\0\0",
                agora,
                &mut s
            ),
            Recv::Drop
        );
    }

    #[test]
    fn pelo_derp_o_wireguard_so_vale_de_um_par_do_mapa() {
        let agora = Instant::now();
        let mut a = lado(true);
        let b = lado(true);
        let v = ligar(&mut a, &b, &[], 2);
        let mut s = Vec::new();
        assert_eq!(
            a.magic.on_derp(2, b.no, b"\x01\0\0\0", agora, &mut s),
            Recv::Wireguard(v)
        );
        let desconhecido = NodeKey::generate().public();
        assert_eq!(
            a.magic
                .on_derp(2, desconhecido, b"\x01\0\0\0", agora, &mut s),
            Recv::Drop
        );
        // Sem região de casa, a última região de onde o par escreveu serve de resposta.
        let mut c = lado(true);
        let vc = ligar(&mut c, &b, &[], 0);
        let mut s = Vec::new();
        c.magic.route(vc, b"x".to_vec(), agora, &mut s);
        assert!(s.is_empty());
        c.magic.on_derp(5, b.no, b"\x01\0\0\0", agora, &mut s);
        c.magic.route(vc, b"x".to_vec(), agora, &mut s);
        assert!(matches!(s.first(), Some(Out::Derp { region: 5, .. })));
    }

    #[test]
    fn par_removido_some_de_todas_as_tabelas() {
        let agora = Instant::now();
        let mut a = lado(true);
        let b = lado(true);
        let v = ligar(&mut a, &b, &["203.0.113.9:41641"], 2);
        a.magic.remove_peer(&b.no);
        let mut s = Vec::new();
        a.magic.route(v, b"x".to_vec(), agora, &mut s);
        assert!(s.is_empty());
        assert_eq!(a.magic.path(&b.no, agora), Path::None);
        assert_eq!(
            a.magic.on_udp(
                "203.0.113.9:41641".parse().unwrap(),
                b"\x01\0\0\0",
                agora,
                &mut s
            ),
            Recv::Drop
        );
        assert_eq!(a.magic.peer_for_virtual(&v), None);
    }
}
