//! O mapa da tailnet (`NetMap`): os pares, as rotas, o DNS e o mapa do DERP, montados a partir
//! das respostas do controle.
//!
//! O controle manda o mapa inteiro no começo de cada conexão e depois só o que mudou
//! (`PeersChanged`, `PeersRemoved`, `PeersChangedPatch`…). O [`NetMap::apply`] aplica cada
//! resposta e devolve um [`Change`] que diz o que mexeu, para o motor refazer só o que precisa.
//!
//! Nada aqui toca a rede: o MagicDNS ([`NetMap::resolve`]) responde só com o que o mapa sabe.
//! Só o IPv4 entra nas rotas e nos nomes por enquanto (o IPv6 é adiável).

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};

use jiff::Timestamp;
use tomni_ui_netstack::Cidr;

use crate::key::NodePublic;
use crate::tailcfg::{DerpMap, DnsConfig, MapResponse, Node, PeerChange, UserProfile};

/// O que mudou depois de aplicar uma resposta do mapa.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    /// O nosso nó mudou (endereços, nome, vencimento da chave).
    pub me: bool,
    /// Os pares que apareceram.
    pub peers_added: Vec<i64>,
    /// Os pares que saíram.
    pub peers_removed: Vec<i64>,
    /// Os pares que continuam e mudaram (chave, endpoints, DERP, online…).
    pub peers_updated: Vec<i64>,
    /// As rotas (os destinos que vão pelo túnel) mudaram.
    pub routes: bool,
    /// O mapa do DERP mudou.
    pub derp: bool,
    /// A configuração de DNS ou o domínio mudou.
    pub dns: bool,
}

impl Change {
    /// Se a resposta não mexeu em nada que o motor use.
    pub fn is_empty(&self) -> bool {
        *self == Change::default()
    }

    /// Se algum par apareceu, saiu ou mudou.
    pub fn peers(&self) -> bool {
        !(self.peers_added.is_empty()
            && self.peers_removed.is_empty()
            && self.peers_updated.is_empty())
    }
}

/// O mapa da tailnet, do nosso ponto de vista.
#[derive(Debug, Clone, Default)]
pub struct NetMap {
    me: Option<Node>,
    peers: BTreeMap<i64, Node>,
    derp_map: DerpMap,
    dns: DnsConfig,
    domain: String,
    users: BTreeMap<i64, UserProfile>,
    control_time: Option<Timestamp>,
    health: Vec<String>,
    accept_routes: bool,
}

impl NetMap {
    /// Um mapa vazio. `accept_routes` diz se as sub-redes que outras máquinas anunciam entram
    /// nas rotas (os endereços da tailnet entram sempre).
    pub fn new(accept_routes: bool) -> Self {
        NetMap {
            accept_routes,
            ..NetMap::default()
        }
    }

    /// O nosso nó, quando o controle já o mandou.
    pub fn me(&self) -> Option<&Node> {
        self.me.as_ref()
    }

    /// Os pares, na ordem do número do nó.
    pub fn peers(&self) -> impl Iterator<Item = &Node> {
        self.peers.values()
    }

    /// O par de um número de nó.
    pub fn peer(&self, id: i64) -> Option<&Node> {
        self.peers.get(&id)
    }

    /// O par de uma chave de nó.
    pub fn peer_by_key(&self, chave: &NodePublic) -> Option<&Node> {
        self.peers.values().find(|n| n.key == *chave)
    }

    /// O mapa do DERP.
    pub fn derp_map(&self) -> &DerpMap {
        &self.derp_map
    }

    /// A configuração de DNS.
    pub fn dns(&self) -> &DnsConfig {
        &self.dns
    }

    /// O nome da tailnet (`minha-tailnet.ts.net`), do `Domain` ou do primeiro domínio de busca.
    pub fn domain(&self) -> &str {
        if self.domain.is_empty() {
            self.dns.domains.first().map_or("", String::as_str)
        } else {
            &self.domain
        }
    }

    /// O perfil de um usuário.
    pub fn user(&self, id: i64) -> Option<&UserProfile> {
        self.users.get(&id)
    }

    /// A hora que o controle disse na última resposta.
    pub fn control_time(&self) -> Option<Timestamp> {
        self.control_time
    }

    /// Os avisos de saúde da última resposta que trouxe algum.
    pub fn health(&self) -> &[String] {
        &self.health
    }

    /// Se as sub-redes anunciadas entram nas rotas.
    pub fn accept_routes(&self) -> bool {
        self.accept_routes
    }

    /// Muda se as sub-redes anunciadas entram nas rotas; devolve `true` se as rotas mudaram.
    pub fn set_accept_routes(&mut self, sim: bool) -> bool {
        if self.accept_routes == sim {
            return false;
        }
        let antes = self.routes();
        self.accept_routes = sim;
        antes != self.routes()
    }

    /// Os nossos endereços IPv4 na tailnet.
    pub fn addresses(&self) -> Vec<Ipv4Addr> {
        self.me.as_ref().map(enderecos_v4).unwrap_or_default()
    }

    /// Aplica uma resposta do mapa e diz o que mudou.
    ///
    /// Um sinal de vida (`KeepAlive`) não muda nada. `Peers` não vazio substitui a lista; os
    /// deltas (`PeersChanged`, `PeersRemoved`, `PeersChangedPatch`, `OnlineChange`,
    /// `PeerSeenChange`) mexem nos pares que já existem.
    pub fn apply(&mut self, resposta: MapResponse) -> Change {
        self.aplicar(resposta, false)
    }

    /// Aplica a primeira resposta de uma conexão nova do mapa: a lista de pares dela é a
    /// lista inteira, mesmo vazia (os pares que não vieram saíram enquanto estávamos fora).
    /// Os pares de `PeersChanged` valem como parte da lista.
    pub fn apply_snapshot(&mut self, mut resposta: MapResponse) -> Change {
        resposta.peers.append(&mut resposta.peers_changed);
        self.aplicar(resposta, true)
    }

    fn aplicar(&mut self, resposta: MapResponse, substitui: bool) -> Change {
        let mut mudou = Change::default();
        if resposta.keep_alive {
            return mudou;
        }
        let rotas_antes = self.routes();

        if let Some(no) = resposta.node
            && self.me.as_ref() != Some(&no)
        {
            self.me = Some(no);
            mudou.me = true;
        }

        if substitui || !resposta.peers.is_empty() {
            let mut novos: BTreeMap<i64, Node> = BTreeMap::new();
            for p in resposta.peers {
                novos.insert(p.id, p);
            }
            for id in self.peers.keys() {
                if !novos.contains_key(id) {
                    mudou.peers_removed.push(*id);
                }
            }
            for (id, novo) in &novos {
                match self.peers.get(id) {
                    None => mudou.peers_added.push(*id),
                    Some(velho) if velho != novo => mudou.peers_updated.push(*id),
                    Some(_) => {}
                }
            }
            self.peers = novos;
        }
        for p in resposta.peers_changed {
            let id = p.id;
            match self.peers.insert(id, p.clone()) {
                None => marcar(&mut mudou.peers_added, id),
                Some(velho) if velho != p => marcar(&mut mudou.peers_updated, id),
                Some(_) => {}
            }
        }
        for id in resposta.peers_removed {
            if self.peers.remove(&id).is_some() {
                marcar(&mut mudou.peers_removed, id);
            }
        }
        for mudanca in resposta.peers_changed_patch {
            let id = mudanca.node_id;
            if let Some(par) = self.peers.get_mut(&id)
                && aplicar_patch(par, mudanca)
            {
                marcar(&mut mudou.peers_updated, id);
            }
        }
        if let Some(online) = resposta.online_change {
            for (id, valor) in online {
                if let Some(par) = self.peers.get_mut(&id)
                    && par.online != Some(valor)
                {
                    par.online = Some(valor);
                    marcar(&mut mudou.peers_updated, id);
                }
            }
        }
        if let Some(vistos) = resposta.peer_seen_change {
            let agora = resposta.control_time.unwrap_or_else(Timestamp::now);
            for (chave, visto) in vistos {
                let Ok(chave) = chave.parse::<NodePublic>() else {
                    continue;
                };
                if let Some(par) = self.peers.values_mut().find(|n| n.key == chave) {
                    par.last_seen = visto.then_some(agora);
                }
            }
        }

        if let Some(mapa) = resposta.derp_map
            && mapa != self.derp_map
        {
            self.derp_map = mapa;
            mudou.derp = true;
        }
        if let Some(dns) = resposta.dns_config
            && dns != self.dns
        {
            self.dns = dns;
            mudou.dns = true;
        }
        if !resposta.domain.is_empty() && resposta.domain != self.domain {
            self.domain = resposta.domain;
            mudou.dns = true;
        }
        for u in resposta.user_profiles {
            self.users.insert(u.id, u);
        }
        if resposta.control_time.is_some() {
            self.control_time = resposta.control_time;
        }
        if !resposta.health.is_empty() {
            self.health = resposta.health;
        }

        if mudou.me || mudou.peers() {
            mudou.routes = rotas_antes != self.routes();
        }
        mudou
    }

    /// Os prefixos que um nó atende: os endereços dele e, com `accept_routes`, as sub-redes
    /// que ele anuncia. O destino padrão (`0.0.0.0/0`, o exit node) e o IPv6 ficam de fora.
    fn prefixos(&self, no: &Node) -> Vec<(Cidr, bool)> {
        let mut saida: Vec<(Cidr, bool)> = Vec::new();
        let mut poe = |texto: &str, primaria: bool| {
            let Some(c) = ler_prefixo(texto) else { return };
            if c.prefix() == 0 || !c.addr().is_ipv4() {
                return;
            }
            let c = Cidr::new(c.network(), c.prefix()).unwrap_or(c);
            if let Some(existente) = saida.iter_mut().find(|(x, _)| *x == c) {
                existente.1 |= primaria;
            } else {
                saida.push((c, primaria));
            }
        };
        for a in &no.addresses {
            poe(a, false);
        }
        if self.accept_routes {
            for a in &no.allowed_ips {
                poe(a, false);
            }
            for a in &no.primary_routes {
                poe(a, true);
            }
        }
        saida
    }

    /// Se o par entra no roteamento: com chave, sem chave vencida.
    pub fn usavel(no: &Node) -> bool {
        !no.key.is_zero() && !no.expired
    }

    /// As faixas que o motor manda a este nó: os endereços dele e, com `accept_routes`, as
    /// sub-redes que ele anuncia (o `AllowedIPs` do WireGuard).
    pub fn prefixes_of(&self, no: &Node) -> Vec<Cidr> {
        self.prefixos(no).into_iter().map(|(c, _)| c).collect()
    }

    /// Os destinos que vão pelo túnel, sem repetição e em ordem.
    pub fn routes(&self) -> Vec<Cidr> {
        let mut todas: Vec<Cidr> = Vec::new();
        for p in self.peers.values().filter(|p| Self::usavel(p)) {
            for (c, _) in self.prefixos(p) {
                if !todas.contains(&c) {
                    todas.push(c);
                }
            }
        }
        todas.sort_by_key(|c| (c.addr(), c.prefix()));
        todas
    }

    /// O par que atende um endereço: o que tem o prefixo mais longo que o contém. Em empate
    /// (duas máquinas anunciam a mesma sub-rede), vale a que o controle marcou como primária
    /// e, depois, a de menor número.
    pub fn peer_for(&self, ip: IpAddr) -> Option<&Node> {
        let mut melhor: Option<(&Node, u8, bool)> = None;
        for p in self.peers.values().filter(|p| Self::usavel(p)) {
            for (c, primaria) in self.prefixos(p) {
                if !c.contains(ip) {
                    continue;
                }
                let melhor_hoje = match melhor {
                    None => true,
                    Some((_, tam, prim)) => {
                        c.prefix() > tam || (c.prefix() == tam && primaria && !prim)
                    }
                };
                if melhor_hoje {
                    melhor = Some((p, c.prefix(), primaria));
                }
            }
        }
        melhor.map(|(p, _, _)| p)
    }

    /// Um par pelo endereço da tailnet dele (sem as sub-redes), vencido ou não: serve para
    /// explicar por que ele não responde.
    pub fn peer_with_address(&self, ip: IpAddr) -> Option<&Node> {
        self.peers.values().find(|p| {
            p.addresses
                .iter()
                .filter_map(|a| ler_prefixo(a))
                .any(|c| c.addr() == ip)
        })
    }

    /// O MagicDNS: o endereço IPv4 de um nome, só com o que o mapa sabe.
    ///
    /// - nome curto (`maquina`, sem ponto): o par cujo nome de máquina (`Hostinfo.Hostname`) ou
    ///   primeiro rótulo do nome completo é igual, sem diferença de maiúsculas;
    /// - nome completo (`maquina.minha-tailnet.ts.net`), com ou sem o ponto final;
    /// - `maquina.{domínio}` para cada domínio de busca do DNS e para o domínio da tailnet;
    /// - os `ExtraRecords` do controle;
    /// - o nome da nossa própria máquina: o nosso endereço.
    ///
    /// Um nome que o mapa não conhece dá `None` (e quem chama segue para o DNS do sistema).
    /// O IPv6 fica de fora.
    pub fn resolve(&self, nome: &str) -> Option<Ipv4Addr> {
        let nome = nome.trim().trim_end_matches('.').to_lowercase();
        if nome.is_empty() {
            return None;
        }
        let nos = || self.me.iter().chain(self.peers.values());

        // 1. O nome completo, exato.
        if let Some(ip) = nos()
            .find(|n| minusculo_sem_ponto(&n.name) == nome)
            .and_then(primeiro_v4)
        {
            return Some(ip);
        }
        // 2. Os registros a mais do controle.
        for r in &self.dns.extra_records {
            if minusculo_sem_ponto(&r.name) == nome
                && matches!(r.tipo.as_str(), "" | "A")
                && let Ok(ip) = r.value.trim().parse::<Ipv4Addr>()
            {
                return Some(ip);
            }
        }
        // 3. O nome curto, direto ou pela busca em um domínio.
        let curto = if nome.contains('.') {
            self.dominios()
                .iter()
                .find_map(|d| nome.strip_suffix(d.as_str())?.strip_suffix('.'))
                .filter(|c| !c.contains('.'))
                .map(str::to_string)
        } else {
            Some(nome)
        }?;
        // Entre dois pares com o mesmo nome curto vale o primeiro (menor número); o nosso
        // nó entra na frente, para o "Testar" achar a própria máquina.
        nos()
            .find(|n| nome_curto(n) == curto || hostname(n) == curto)
            .and_then(primeiro_v4)
    }

    /// Os domínios em que um nome curto vale: os de busca do DNS e o da tailnet.
    fn dominios(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .dns
            .domains
            .iter()
            .map(|d| minusculo_sem_ponto(d))
            .collect();
        let d = minusculo_sem_ponto(self.domain());
        if !d.is_empty() && !v.contains(&d) {
            v.push(d);
        }
        v.retain(|d| !d.is_empty());
        v
    }
}

fn marcar(lista: &mut Vec<i64>, id: i64) {
    if !lista.contains(&id) {
        lista.push(id);
    }
}

/// Aplica uma mudança parcial; devolve `true` se algo mudou de fato.
fn aplicar_patch(par: &mut Node, m: PeerChange) -> bool {
    let antes = par.clone();
    if let Some(r) = m.derp_region {
        par.home_derp = r;
    }
    if let Some(e) = m.endpoints {
        par.endpoints = e.iter().filter_map(|t| t.parse().ok()).collect();
    }
    if let Some(k) = m.key {
        par.key = k;
    }
    if let Some(k) = m.disco_key {
        par.disco_key = k;
    }
    if let Some(o) = m.online {
        par.online = Some(o);
    }
    if let Some(t) = m.last_seen {
        par.last_seen = Some(t);
    }
    if let Some(t) = m.key_expiry {
        par.key_expiry = Some(t);
    }
    *par != antes
}

/// Lê `"100.64.0.1/32"` (ou um IP sem prefixo, que vale `/32` ou `/128`).
fn ler_prefixo(texto: &str) -> Option<Cidr> {
    let texto = texto.trim();
    if texto.contains('/') {
        texto.parse().ok()
    } else {
        let ip: IpAddr = texto.parse().ok()?;
        Cidr::new(ip, if ip.is_ipv4() { 32 } else { 128 })
    }
}

fn enderecos_v4(no: &Node) -> Vec<Ipv4Addr> {
    no.addresses
        .iter()
        .filter_map(|a| ler_prefixo(a))
        .filter_map(|c| match c.addr() {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        })
        .collect()
}

fn primeiro_v4(no: &Node) -> Option<Ipv4Addr> {
    enderecos_v4(no).into_iter().next()
}

fn minusculo_sem_ponto(s: &str) -> String {
    s.trim_end_matches('.').to_lowercase()
}

/// O primeiro rótulo do nome completo, em minúsculas.
fn nome_curto(no: &Node) -> String {
    minusculo_sem_ponto(&no.name)
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// O nome de máquina que o nó diz ter, em minúsculas.
fn hostname(no: &Node) -> String {
    no.hostinfo
        .as_ref()
        .map(|h| h.hostname.to_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::key::NodeKey;
    use crate::tailcfg::{DnsRecord, Hostinfo};

    fn chave(n: u8) -> NodePublic {
        NodeKey::from_bytes([n; 32]).public()
    }

    fn no(id: i64, nome: &str, ip: &str) -> Node {
        Node {
            id,
            name: format!("{nome}.exemplo.ts.net."),
            key: chave(id as u8),
            addresses: vec![format!("{ip}/32")],
            allowed_ips: vec![format!("{ip}/32")],
            hostinfo: Some(Hostinfo {
                hostname: nome.to_string(),
                ..Hostinfo::default()
            }),
            ..Node::default()
        }
    }

    fn mapa(nos: Vec<Node>) -> MapResponse {
        MapResponse {
            node: Some(no(1, "eu", "100.64.0.1")),
            peers: nos,
            ..MapResponse::default()
        }
    }

    #[test]
    fn o_mapa_inteiro_substitui_os_pares() {
        let mut m = NetMap::new(true);
        let c = m.apply(mapa(vec![
            no(2, "a", "100.64.0.2"),
            no(3, "b", "100.64.0.3"),
        ]));
        assert!(c.me);
        assert_eq!(c.peers_added, vec![2, 3]);
        assert!(c.routes);
        assert_eq!(m.peers().count(), 2);
        let c = m.apply(mapa(vec![
            no(3, "b", "100.64.0.3"),
            no(4, "c", "100.64.0.4"),
        ]));
        assert_eq!(c.peers_added, vec![4]);
        assert_eq!(c.peers_removed, vec![2]);
        assert!(c.peers_updated.is_empty());
        assert!(c.routes);
        assert!(m.peer(2).is_none());
    }

    #[test]
    fn o_sinal_de_vida_nao_muda_nada() {
        let mut m = NetMap::new(true);
        m.apply(mapa(vec![no(2, "a", "100.64.0.2")]));
        let c = m.apply(MapResponse {
            keep_alive: true,
            ..MapResponse::default()
        });
        assert!(c.is_empty());
        assert_eq!(m.peers().count(), 1);
    }

    #[test]
    fn deltas_de_pares_novos_trocados_e_removidos() {
        let mut m = NetMap::new(true);
        m.apply(mapa(vec![no(2, "a", "100.64.0.2")]));
        let mut a2 = no(2, "a", "100.64.0.2");
        a2.home_derp = 7;
        let c = m.apply(MapResponse {
            peers_changed: vec![a2, no(3, "b", "100.64.0.3")],
            ..MapResponse::default()
        });
        assert_eq!(c.peers_added, vec![3]);
        assert_eq!(c.peers_updated, vec![2]);
        assert_eq!(m.peer(2).unwrap().home_derp, 7);
        let c = m.apply(MapResponse {
            peers_removed: vec![2, 99],
            ..MapResponse::default()
        });
        assert_eq!(c.peers_removed, vec![2]);
        assert!(m.peer(2).is_none());
        assert!(c.routes);
    }

    #[test]
    fn o_patch_muda_so_os_campos_presentes() {
        let mut m = NetMap::new(true);
        m.apply(mapa(vec![no(2, "a", "100.64.0.2")]));
        let nova = chave(50);
        let c = m.apply(MapResponse {
            peers_changed_patch: vec![
                PeerChange {
                    node_id: 2,
                    derp_region: Some(9),
                    endpoints: Some(vec!["192.0.2.7:41641".into(), "lixo".into()]),
                    online: Some(true),
                    key: Some(nova),
                    ..PeerChange::default()
                },
                // Um par que não conhecemos é ignorado.
                PeerChange {
                    node_id: 77,
                    online: Some(true),
                    ..PeerChange::default()
                },
            ],
            ..MapResponse::default()
        });
        assert_eq!(c.peers_updated, vec![2]);
        let p = m.peer(2).unwrap();
        assert_eq!(p.home_derp, 9);
        assert_eq!(p.online, Some(true));
        assert_eq!(p.key, nova);
        assert_eq!(p.endpoints, vec!["192.0.2.7:41641".parse().unwrap()]);
        assert_eq!(p.name, "a.exemplo.ts.net.");
        // Repetir o mesmo patch não é mudança.
        let c = m.apply(MapResponse {
            peers_changed_patch: vec![PeerChange {
                node_id: 2,
                online: Some(true),
                ..PeerChange::default()
            }],
            ..MapResponse::default()
        });
        assert!(c.is_empty());
    }

    #[test]
    fn online_e_visto_por_ultimo() {
        let mut m = NetMap::new(true);
        let a = no(2, "a", "100.64.0.2");
        let chave_a = a.key;
        m.apply(mapa(vec![a]));
        let mut online = BTreeMap::new();
        online.insert(2, false);
        let mut vistos = BTreeMap::new();
        vistos.insert(chave_a.to_string(), true);
        let c = m.apply(MapResponse {
            online_change: Some(online),
            peer_seen_change: Some(vistos),
            ..MapResponse::default()
        });
        assert_eq!(c.peers_updated, vec![2]);
        let p = m.peer(2).unwrap();
        assert_eq!(p.online, Some(false));
        assert!(p.last_seen.is_some());
    }

    #[test]
    fn as_rotas_sao_os_enderecos_dos_pares() {
        let mut m = NetMap::new(true);
        m.apply(mapa(vec![
            no(2, "a", "100.64.0.2"),
            no(3, "b", "100.64.0.3"),
        ]));
        let rotas: Vec<String> = m.routes().iter().map(|c| c.to_string()).collect();
        assert_eq!(rotas, ["100.64.0.2/32", "100.64.0.3/32"]);
        assert_eq!(m.addresses(), vec![Ipv4Addr::new(100, 64, 0, 1)]);
    }

    #[test]
    fn sub_rede_anunciada_entra_com_accept_routes() {
        let mut roteador = no(2, "roteador", "100.64.0.2");
        roteador.allowed_ips = vec!["100.64.0.2/32".into(), "192.168.50.0/24".into()];
        roteador.primary_routes = vec!["192.168.50.0/24".into()];
        let mut m = NetMap::new(true);
        m.apply(mapa(vec![roteador.clone(), no(3, "b", "100.64.0.3")]));
        let rotas: Vec<String> = m.routes().iter().map(|c| c.to_string()).collect();
        assert!(rotas.contains(&"192.168.50.0/24".to_string()), "{rotas:?}");
        assert_eq!(
            m.peer_for("192.168.50.9".parse().unwrap()).map(|n| n.id),
            Some(2)
        );

        // Sem aceitar as rotas, só os endereços da tailnet.
        let mut m = NetMap::new(false);
        m.apply(mapa(vec![roteador, no(3, "b", "100.64.0.3")]));
        let rotas: Vec<String> = m.routes().iter().map(|c| c.to_string()).collect();
        assert_eq!(rotas, ["100.64.0.2/32", "100.64.0.3/32"]);
        assert!(m.peer_for("192.168.50.9".parse().unwrap()).is_none());
        assert_eq!(
            m.peer_for("100.64.0.3".parse().unwrap()).map(|n| n.id),
            Some(3)
        );
    }

    #[test]
    fn a_rota_padrao_do_exit_node_e_ignorada() {
        let mut saida = no(2, "saida", "100.64.0.2");
        saida.allowed_ips = vec!["100.64.0.2/32".into(), "0.0.0.0/0".into(), "::/0".into()];
        saida.primary_routes = vec!["0.0.0.0/0".into()];
        let mut m = NetMap::new(true);
        m.apply(mapa(vec![saida]));
        let rotas: Vec<String> = m.routes().iter().map(|c| c.to_string()).collect();
        assert_eq!(rotas, ["100.64.0.2/32"]);
        assert!(m.peer_for("8.8.8.8".parse().unwrap()).is_none());
    }

    #[test]
    fn o_prefixo_mais_longo_ganha() {
        let mut largo = no(2, "largo", "100.64.0.2");
        largo.allowed_ips = vec!["100.64.0.2/32".into(), "10.0.0.0/8".into()];
        let mut estreito = no(3, "estreito", "100.64.0.3");
        estreito.allowed_ips = vec!["100.64.0.3/32".into(), "10.1.0.0/16".into()];
        let mut m = NetMap::new(true);
        m.apply(mapa(vec![largo, estreito]));
        let id = |ip: &str| m.peer_for(ip.parse().unwrap()).map(|n| n.id);
        assert_eq!(id("10.1.2.3"), Some(3));
        assert_eq!(id("10.2.0.1"), Some(2));
        assert_eq!(id("11.0.0.1"), None);
    }

    #[test]
    fn empate_de_sub_rede_vale_a_primaria() {
        let mut a = no(2, "a", "100.64.0.2");
        a.allowed_ips = vec!["100.64.0.2/32".into(), "10.9.0.0/16".into()];
        let mut b = no(3, "b", "100.64.0.3");
        b.allowed_ips = vec!["100.64.0.3/32".into(), "10.9.0.0/16".into()];
        b.primary_routes = vec!["10.9.0.0/16".into()];
        let mut m = NetMap::new(true);
        m.apply(mapa(vec![a, b]));
        assert_eq!(
            m.peer_for("10.9.1.1".parse().unwrap()).map(|n| n.id),
            Some(3)
        );
    }

    #[test]
    fn rota_nova_no_meio_do_caminho() {
        let mut m = NetMap::new(true);
        m.apply(mapa(vec![no(2, "r", "100.64.0.2")]));
        let mut r = no(2, "r", "100.64.0.2");
        r.allowed_ips.push("172.16.0.0/12".into());
        let c = m.apply(MapResponse {
            peers_changed: vec![r],
            ..MapResponse::default()
        });
        assert!(c.routes);
        assert_eq!(c.peers_updated, vec![2]);
        assert_eq!(m.routes().len(), 2);
    }

    #[test]
    fn o_par_vencido_sai_das_rotas() {
        let mut m = NetMap::new(true);
        let mut v = no(2, "a", "100.64.0.2");
        v.expired = true;
        m.apply(mapa(vec![v, no(3, "b", "100.64.0.3")]));
        assert_eq!(m.routes().len(), 1);
        assert!(m.peer_for("100.64.0.2".parse().unwrap()).is_none());
        // Mas dá para achar o par para explicar a falha.
        assert_eq!(
            m.peer_with_address("100.64.0.2".parse().unwrap())
                .map(|n| n.id),
            Some(2)
        );
    }

    fn mapa_dns() -> NetMap {
        let mut m = NetMap::new(true);
        let mut resp = mapa(vec![
            no(2, "Servidor", "100.64.0.2"),
            no(3, "banco", "100.64.0.3"),
        ]);
        // O nome de máquina que o nó diz ter pode diferir do rótulo do nome completo.
        resp.peers[1].hostinfo.as_mut().unwrap().hostname = "db-principal".into();
        resp.dns_config = Some(DnsConfig {
            domains: vec!["exemplo.ts.net".into(), "interno.empresa".into()],
            proxied: true,
            extra_records: vec![
                DnsRecord {
                    name: "painel.exemplo.ts.net".into(),
                    tipo: String::new(),
                    value: "100.64.0.9".into(),
                },
                DnsRecord {
                    name: "v6.exemplo.ts.net".into(),
                    tipo: "AAAA".into(),
                    value: "fd7a::1".into(),
                },
            ],
            ..DnsConfig::default()
        });
        m.apply(resp);
        m
    }

    #[test]
    fn magicdns_resolve_o_nome_curto_sem_diferenca_de_maiusculas() {
        let m = mapa_dns();
        let ip = |n: &str| m.resolve(n);
        assert_eq!(ip("servidor"), Some(Ipv4Addr::new(100, 64, 0, 2)));
        assert_eq!(ip("SERVIDOR"), Some(Ipv4Addr::new(100, 64, 0, 2)));
        // Pelo nome de máquina do Hostinfo e pelo rótulo do nome completo.
        assert_eq!(ip("db-principal"), Some(Ipv4Addr::new(100, 64, 0, 3)));
        assert_eq!(ip("banco"), Some(Ipv4Addr::new(100, 64, 0, 3)));
    }

    #[test]
    fn magicdns_resolve_o_nome_completo_com_e_sem_ponto() {
        let m = mapa_dns();
        assert_eq!(
            m.resolve("servidor.exemplo.ts.net"),
            Some(Ipv4Addr::new(100, 64, 0, 2))
        );
        assert_eq!(
            m.resolve("Servidor.Exemplo.TS.net."),
            Some(Ipv4Addr::new(100, 64, 0, 2))
        );
    }

    #[test]
    fn magicdns_usa_cada_dominio_de_busca() {
        let m = mapa_dns();
        assert_eq!(
            m.resolve("servidor.interno.empresa"),
            Some(Ipv4Addr::new(100, 64, 0, 2))
        );
        assert_eq!(
            m.resolve("db-principal.interno.empresa"),
            Some(Ipv4Addr::new(100, 64, 0, 3))
        );
        // Um domínio que não é de busca não vale.
        assert_eq!(m.resolve("servidor.outro.dominio"), None);
        // Nem dois rótulos antes do domínio.
        assert_eq!(m.resolve("a.servidor.interno.empresa"), None);
    }

    #[test]
    fn magicdns_responde_pelos_registros_a_mais_e_pelo_proprio_nome() {
        let m = mapa_dns();
        assert_eq!(
            m.resolve("painel.exemplo.ts.net."),
            Some(Ipv4Addr::new(100, 64, 0, 9))
        );
        // Registro AAAA não vira IPv4.
        assert_eq!(m.resolve("v6.exemplo.ts.net"), None);
        // O nosso nome devolve o nosso endereço.
        assert_eq!(m.resolve("eu"), Some(Ipv4Addr::new(100, 64, 0, 1)));
        assert_eq!(
            m.resolve("eu.exemplo.ts.net"),
            Some(Ipv4Addr::new(100, 64, 0, 1))
        );
    }

    #[test]
    fn magicdns_nome_desconhecido_ou_vazio_e_none() {
        let m = mapa_dns();
        assert_eq!(m.resolve("nao-existe"), None);
        assert_eq!(m.resolve("google.com"), None);
        assert_eq!(m.resolve(""), None);
        assert_eq!(m.resolve("."), None);
        // Um IP não é nome do mapa.
        assert_eq!(m.resolve("100.64.0.2"), None);
    }

    #[test]
    fn o_dominio_vem_do_mapa_ou_da_busca() {
        let mut m = NetMap::new(true);
        assert_eq!(m.domain(), "");
        m.apply(MapResponse {
            dns_config: Some(DnsConfig {
                domains: vec!["a.ts.net".into()],
                ..DnsConfig::default()
            }),
            ..MapResponse::default()
        });
        assert_eq!(m.domain(), "a.ts.net");
        let c = m.apply(MapResponse {
            domain: "b.ts.net".into(),
            ..MapResponse::default()
        });
        assert!(c.dns);
        assert_eq!(m.domain(), "b.ts.net");
    }

    #[test]
    fn o_mapa_do_derp_e_o_dns_so_mudam_quando_diferem() {
        let mut m = NetMap::new(true);
        let derp = DerpMap {
            omit_default_regions: true,
            ..DerpMap::default()
        };
        let c = m.apply(MapResponse {
            derp_map: Some(derp.clone()),
            ..MapResponse::default()
        });
        assert!(c.derp);
        let c = m.apply(MapResponse {
            derp_map: Some(derp),
            ..MapResponse::default()
        });
        assert!(!c.derp);
    }

    #[test]
    fn o_primeiro_mapa_de_uma_conexao_nova_vale_como_a_lista_inteira() {
        let mut m = NetMap::new(true);
        m.apply(mapa(vec![
            no(2, "a", "100.64.0.2"),
            no(3, "b", "100.64.0.3"),
        ]));
        // Sem nenhum par: todos os que tínhamos saíram.
        let c = m.apply_snapshot(MapResponse::default());
        assert_eq!(c.peers_removed, vec![2, 3]);
        assert_eq!(m.peers().count(), 0);
        assert!(m.me().is_some());
        // Com `PeersChanged` no lugar de `Peers`, ainda vale como a lista.
        let c = m.apply_snapshot(MapResponse {
            peers_changed: vec![no(4, "c", "100.64.0.4")],
            ..MapResponse::default()
        });
        assert_eq!(c.peers_added, vec![4]);
        assert_eq!(m.peers().count(), 1);
    }

    #[test]
    fn mudar_accept_routes_diz_se_as_rotas_mudaram() {
        let mut r = no(2, "r", "100.64.0.2");
        r.allowed_ips.push("192.168.1.0/24".into());
        let mut m = NetMap::new(false);
        m.apply(mapa(vec![r]));
        assert_eq!(m.routes().len(), 1);
        assert!(m.set_accept_routes(true));
        assert_eq!(m.routes().len(), 2);
        assert!(!m.set_accept_routes(true));
    }
}
