//! Os relays: quando dois nós não se alcançam direto (os dois atrás de CGNAT, por exemplo), um
//! terceiro, com `relay.am_relay`, passa os pacotes de um ao outro. O túnel de ponta a ponta
//! continua cifrado entre os dois; o relay só vê o pacote já cifrado, autenticado no túnel com
//! cada ponta.
//!
//! Portado do Nebula (github.com/slackhq/nebula, licença MIT, `relay_manager.go`, `hostmap.go`,
//! `outside.go`, `inside.go` e `connection_manager.go`). O nó que quer chegar a um par pede ao
//! relay (`CreateRelayRequest`), o relay pede ao par, e as respostas (`CreateRelayResponse`)
//! voltam pelo mesmo caminho; a partir daí o aperto de mão e os dados passam dentro de pacotes
//! `MESSAGE_RELAY`.

use std::net::{IpAddr, SocketAddr};

use super::{Engine, Relay, RelayKind, RelayState, Via};
use crate::cert::Version;
use crate::control::{Control, ControlType};
use crate::header::{self, Header, MessageType};
use crate::noise::TAG_LEN;
use crate::underlay::Underlay;

impl<U: Underlay> Engine<U> {
    /// O nó usa relays para chegar aos pares (`relay.use_relays`; quem é relay não usa).
    pub(super) fn use_relays(&self) -> bool {
        self.cfg.relay.use_relays && !self.am_relay
    }

    /// Os relays pelos quais este nó aceita ser alcançado, para avisar o lighthouse.
    pub(super) fn relays_for_me(&self) -> Vec<IpAddr> {
        if self.am_relay {
            return Vec::new();
        }
        self.cfg.relay.relays.clone()
    }

    /// Abre o túnel com os relays configurados, para que os pares nos alcancem por eles.
    pub(super) fn connect_relays(&mut self) {
        for relay in self.relays_for_me() {
            if !self.primary.contains_key(&relay) {
                self.start_handshake(relay);
            }
        }
    }

    /// Um índice local de relay que ninguém usa (nunca zero). Os índices de relay são outro
    /// espaço que os dos túneis, como no Nebula.
    fn alloc_relay_index(&self) -> Option<u32> {
        for _ in 0..32 {
            let mut bytes = [0u8; 4];
            let _ = getrandom::fill(&mut bytes);
            let escolhido = u32::from_be_bytes(bytes);
            if escolhido != 0 && !self.relay_index.contains_key(&escolhido) {
                return Some(escolhido);
            }
        }
        None
    }

    /// Guarda um relay com `peer` no túnel `host` e o torna o principal do par (só o principal
    /// é procurado pelos relays). Devolve o índice local.
    fn add_relay(
        &mut self,
        host: u32,
        peer: IpAddr,
        remote_index: Option<u32>,
        kind: RelayKind,
        state: RelayState,
    ) -> Option<u32> {
        let indice = self.alloc_relay_index()?;
        let h = self.hosts.get_mut(&host)?;
        let anterior = h.relay_for.insert(
            peer,
            Relay {
                kind,
                state,
                local_index: indice,
                remote_index: remote_index.unwrap_or(0),
                peer,
            },
        );
        let enderecos = h.addrs.clone();
        if let Some(anterior) = anterior {
            self.relay_index.remove(&anterior.local_index);
        }
        self.relay_index.insert(indice, host);
        for addr in enderecos {
            self.primary.insert(addr, host);
        }
        Some(indice)
    }

    /// Muda o estado do relay com `peer` no túnel `host`.
    fn set_relay_state(&mut self, host: u32, peer: IpAddr, state: RelayState) {
        if let Some(r) = self
            .hosts
            .get_mut(&host)
            .and_then(|h| h.relay_for.get_mut(&peer))
        {
            r.state = state;
        }
    }

    /// O relay estabelecido, num túnel com `relay_vpn` (o principal primeiro), que leva a um dos
    /// endereços de `targets` (o `QueryVpnAddrsRelayFor`).
    fn query_relay_for(&self, targets: &[IpAddr], relay_vpn: IpAddr) -> Option<(u32, Relay)> {
        let principal = self.primary.get(&relay_vpn).copied();
        let outros = self
            .hosts
            .values()
            .filter(|h| h.vpn == relay_vpn && Some(h.local_index) != principal)
            .map(|h| h.local_index);
        for indice in principal.into_iter().chain(outros) {
            let Some(h) = self.hosts.get(&indice) else {
                continue;
            };
            for alvo in targets {
                if let Some(r) = h.relay_for.get(alvo)
                    && r.state == RelayState::Established
                {
                    return Some((indice, *r));
                }
            }
        }
        None
    }

    /// Manda `inner` (um pacote já pronto do túnel de ponta a ponta, ou o primeiro do aperto de
    /// mão) pelo relay de índice local `relay` do túnel `host` (o `SendVia`).
    pub(super) fn send_via(&mut self, host: u32, relay: u32, inner: &[u8]) -> bool {
        let Some(h) = self.hosts.get_mut(&host) else {
            return false;
        };
        let Some(remoto) = h.remote else {
            return false;
        };
        let Some(r) = h.relay_for.values().find(|r| r.local_index == relay) else {
            return false;
        };
        let indice_remoto = r.remote_index;
        let Some(pacote) = h.tunnel.seal_relay(indice_remoto, inner) else {
            return false;
        };
        h.out_traffic = true;
        self.raw_send(&pacote, remoto);
        true
    }

    /// Manda um pacote já cifrado do túnel `indice` pelo primeiro relay que leva ao par. Os
    /// relays que não levam mais saem da lista.
    pub(super) fn send_by_relays(&mut self, indice: u32, pacote: &[u8]) -> bool {
        let Some(h) = self.hosts.get(&indice) else {
            return false;
        };
        let enderecos = h.addrs.clone();
        let relays = h.relays.clone();
        for relay in relays {
            match self.query_relay_for(&enderecos, relay) {
                Some((host, r)) => {
                    if self.send_via(host, r.local_index, pacote) {
                        return true;
                    }
                }
                None => {
                    if let Some(h) = self.hosts.get_mut(&indice) {
                        h.relays.retain(|x| *x != relay);
                    }
                }
            }
        }
        false
    }

    /// Responde por onde o pacote chegou. Pelo relay, ele passa a valer: o par já o usou.
    pub(super) fn reply(&mut self, via: Via, bytes: &[u8]) {
        match via {
            Via::Udp(de) => self.raw_send(bytes, de),
            Via::Relay { host, relay, .. } => {
                if let Some(r) = self
                    .hosts
                    .get_mut(&host)
                    .and_then(|h| h.relay_for.values_mut().find(|r| r.local_index == relay))
                {
                    r.state = RelayState::Established;
                }
                self.send_via(host, relay, bytes);
            }
        }
    }

    /// Manda um pedido de relay de `from` até `to` ao nó do túnel `host`, no formato do
    /// certificado dele.
    fn send_relay_request(&mut self, host: u32, indice: u32, from: IpAddr, to: IpAddr) {
        let Some(versao) = self.hosts.get(&host).map(|h| h.peer_cert.version()) else {
            return;
        };
        let Some(pedido) =
            Control::new(ControlType::CreateRelayRequest, versao, indice, 0, from, to)
        else {
            return;
        };
        self.send_to_host(host, MessageType::Control, 0, &pedido.marshal());
    }

    /// Tenta chegar a `vpn` pelos relays que o lighthouse contou (o `StartRelays`): abre o túnel
    /// com o relay, pede o relay, ou manda o primeiro pacote do aperto de mão por ele.
    pub(super) fn start_relays(&mut self, vpn: IpAddr) {
        if !self.use_relays() {
            return;
        }
        let relays = self
            .remotes
            .get(&vpn)
            .map(|r| r.relays())
            .unwrap_or_default();
        let Some(&meu) = self.my_addrs.first() else {
            return;
        };
        let Some(stage0) = self.pending.get(&vpn).map(|p| p.stage0.clone()) else {
            return;
        };
        for relay in relays {
            if relay == vpn || self.my_addrs.contains(&relay) {
                continue;
            }
            let host = self
                .primary
                .get(&relay)
                .copied()
                .filter(|i| self.hosts.get(i).is_some_and(|h| h.remote.is_some()));
            let Some(host) = host else {
                self.start_handshake(relay);
                continue;
            };
            let existente = self
                .hosts
                .get(&host)
                .and_then(|h| h.relay_for.get(&vpn))
                .copied();
            match existente {
                None => {
                    if let Some(indice) =
                        self.add_relay(host, vpn, None, RelayKind::Terminal, RelayState::Requested)
                    {
                        self.send_relay_request(host, indice, meu, vpn);
                    }
                }
                Some(r) => match r.state {
                    RelayState::Established => {
                        if self.send_via(host, r.local_index, &stage0)
                            && let Some(p) = self.pending.get_mut(&vpn)
                        {
                            p.sent_any = true;
                        }
                    }
                    RelayState::Disestablished | RelayState::Requested => {
                        self.set_relay_state(host, vpn, RelayState::Requested);
                        self.send_relay_request(host, r.local_index, meu, vpn);
                    }
                    RelayState::PeerRequested => {}
                },
            }
        }
    }

    /// Um pacote `MESSAGE_RELAY` chegou pelo UDP: confere no túnel com quem o mandou e entrega o
    /// pacote de dentro (se somos a ponta) ou o passa adiante (se somos o relay).
    pub(super) fn on_relay_packet(&mut self, pacote: &[u8], de: SocketAddr, h: Header) {
        let Some(&indice) = self.relay_index.get(&h.remote_index) else {
            return;
        };
        let Some(host) = self.hosts.get_mut(&indice) else {
            return;
        };
        if host.tunnel.open_relay(pacote, h.counter).is_err() {
            return;
        }
        host.in_traffic = true;
        let through = host.vpn;
        let enderecos = host.addrs.clone();
        let Some(relay) = host
            .relay_for
            .values()
            .find(|r| r.local_index == h.remote_index)
            .copied()
        else {
            return;
        };
        self.roam(indice, de);
        let dentro = &pacote[header::LEN..pacote.len() - TAG_LEN];
        match relay.kind {
            RelayKind::Terminal => {
                let Some(hd) = Header::parse(dentro) else {
                    return;
                };
                // Relay dentro de relay não existe.
                if hd.version != header::VERSION
                    || (hd.kind == MessageType::Message && hd.subtype == header::MESSAGE_RELAY)
                {
                    return;
                }
                let via = Via::Relay {
                    host: indice,
                    relay: relay.local_index,
                    through,
                };
                self.on_outside(dentro, hd, via);
            }
            RelayKind::Forwarding => {
                let Some((alvo, r)) = self.query_relay_for(&enderecos, relay.peer) else {
                    return;
                };
                if r.kind == RelayKind::Forwarding {
                    self.send_via(alvo, r.local_index, dentro);
                }
            }
        }
    }

    /// Uma mensagem de controle chegou pelo túnel `indice`.
    pub(super) fn on_control(&mut self, indice: u32, carga: &[u8]) {
        let Some(msg) = Control::unmarshal(carga) else {
            return;
        };
        let Some((versao, from, to)) = msg.version_from_to() else {
            return;
        };
        match msg.kind {
            ControlType::CreateRelayRequest => {
                self.on_relay_request(indice, versao, &msg, from, to);
            }
            ControlType::CreateRelayResponse => self.on_relay_response(indice, versao, &msg, to),
            ControlType::Other(_) => {}
        }
    }

    /// O `handleCreateRelayRequest`: somos a ponta (e respondemos) ou o relay (e pedimos ao
    /// par).
    fn on_relay_request(
        &mut self,
        indice: u32,
        versao: Version,
        msg: &Control,
        from: IpAddr,
        to: IpAddr,
    ) {
        if self.my_addrs.contains(&from) {
            return;
        }
        if self.my_addrs.contains(&to) {
            let existente = self
                .hosts
                .get(&indice)
                .and_then(|h| h.relay_for.get(&from))
                .copied();
            match existente {
                Some(r) => match r.state {
                    RelayState::Requested => {
                        if let Some(r) = self
                            .hosts
                            .get_mut(&indice)
                            .and_then(|h| h.relay_for.get_mut(&from))
                        {
                            r.remote_index = msg.initiator_relay_index;
                            r.state = RelayState::Established;
                        }
                    }
                    RelayState::Established => {
                        if r.remote_index != msg.initiator_relay_index {
                            return;
                        }
                    }
                    RelayState::Disestablished => {
                        if r.remote_index != msg.initiator_relay_index {
                            return;
                        }
                        self.set_relay_state(indice, from, RelayState::Established);
                    }
                    RelayState::PeerRequested => {}
                },
                None => {
                    let novo = self.add_relay(
                        indice,
                        from,
                        Some(msg.initiator_relay_index),
                        RelayKind::Terminal,
                        RelayState::Established,
                    );
                    if novo.is_none() {
                        return;
                    }
                }
            }
            let Some(r) = self
                .hosts
                .get(&indice)
                .and_then(|h| h.relay_for.get(&from))
                .copied()
            else {
                return;
            };
            let Some(resposta) = Control::new(
                ControlType::CreateRelayResponse,
                versao,
                r.remote_index,
                r.local_index,
                from,
                to,
            ) else {
                return;
            };
            self.send_to_host(indice, MessageType::Control, 0, &resposta.marshal());
            return;
        }

        // O alvo é outro: somos o relay.
        if !self.am_relay {
            return;
        }
        let Some(quem_pede) = self.hosts.get(&indice) else {
            return;
        };
        if !quem_pede.addrs.contains(&from) {
            return;
        }
        let quem_pede_vpn = quem_pede.vpn;
        let Some(&par) = self.primary.get(&to) else {
            // Abre o túnel com o alvo: o próximo pedido o encontra pronto.
            self.start_handshake(to);
            return;
        };
        if self.hosts.get(&par).is_none_or(|h| h.remote.is_none()) {
            // Só se faz relay para quem tem caminho direto.
            return;
        }
        let existente = self
            .hosts
            .get(&par)
            .and_then(|h| h.relay_for.get(&from))
            .copied();
        let indice_par = match existente {
            Some(r) => r.local_index,
            None => {
                match self.add_relay(
                    par,
                    from,
                    None,
                    RelayKind::Forwarding,
                    RelayState::Requested,
                ) {
                    Some(i) => i,
                    None => return,
                }
            }
        };
        self.set_relay_state(par, from, RelayState::Requested);
        if let Some(pedido) = Control::new(
            ControlType::CreateRelayRequest,
            versao,
            indice_par,
            0,
            quem_pede_vpn,
            to,
        ) {
            self.send_to_host(par, MessageType::Control, 0, &pedido.marshal());
        } else {
            return;
        }
        let ja_tem = self
            .hosts
            .get(&indice)
            .is_some_and(|h| h.relay_for.contains_key(&to));
        if !ja_tem {
            self.add_relay(
                indice,
                to,
                Some(msg.initiator_relay_index),
                RelayKind::Forwarding,
                RelayState::PeerRequested,
            );
        }
    }

    /// O `handleCreateRelayResponse`: o relay que pedimos está pronto. Na ponta, o aperto de mão
    /// que espera já passa por ele; no relay, a resposta segue para quem pediu.
    fn on_relay_response(&mut self, indice: u32, versao: Version, msg: &Control, to: IpAddr) {
        let Some(relay) = self.hosts.get_mut(&indice).and_then(|h| {
            h.relay_for
                .values_mut()
                .find(|r| r.local_index == msg.initiator_relay_index)
        }) else {
            return;
        };
        relay.remote_index = msg.responder_relay_index;
        relay.state = RelayState::Established;
        let relay = *relay;
        match relay.kind {
            RelayKind::Terminal => {
                // Não espera a próxima tentativa: o primeiro pacote já vai pelo relay.
                let stage0 = self.pending.get(&relay.peer).map(|p| p.stage0.clone());
                if let Some(stage0) = stage0
                    && self.send_via(indice, relay.local_index, &stage0)
                    && let Some(p) = self.pending.get_mut(&relay.peer)
                {
                    p.sent_any = true;
                }
            }
            RelayKind::Forwarding => {
                let Some(&par) = self.primary.get(&relay.peer) else {
                    return;
                };
                let Some(h) = self.hosts.get(&par) else {
                    return;
                };
                let par_vpn = h.vpn;
                let Some(r) = h.relay_for.get(&to).copied() else {
                    return;
                };
                if r.state == RelayState::Requested {
                    // Também pedimos a esse par e falta ele responder.
                    return;
                }
                self.set_relay_state(par, to, RelayState::Established);
                let Some(resposta) = Control::new(
                    ControlType::CreateRelayResponse,
                    versao,
                    r.remote_index,
                    r.local_index,
                    par_vpn,
                    to,
                ) else {
                    return;
                };
                self.send_to_host(par, MessageType::Control, 0, &resposta.marshal());
            }
        }
    }

    /// Um túnel saiu do mapa: os índices de relay dele somem. Se era o último com o par, os
    /// relays que dependiam dele deixam de valer nos outros túneis; se há outro, os relays
    /// passam para ele (pedidos de novo, como no `migrateRelayUsed`).
    pub(super) fn relays_after_delete(
        &mut self,
        vpn: IpAddr,
        relay_for: Vec<Relay>,
        relays: &[IpAddr],
        sucessor: Option<u32>,
    ) {
        for r in &relay_for {
            self.relay_index.remove(&r.local_index);
        }
        if let Some(novo) = sucessor {
            self.migrate_relays(&relay_for, novo);
            return;
        }
        let mut afetados: Vec<IpAddr> = relays.to_vec();
        afetados.extend(
            relay_for
                .iter()
                .filter(|r| r.kind == RelayKind::Forwarding)
                .map(|r| r.peer),
        );
        for h in self.hosts.values_mut() {
            if afetados.contains(&h.vpn)
                && let Some(r) = h.relay_for.get_mut(&vpn)
            {
                r.state = RelayState::Disestablished;
            }
        }
    }

    /// Pede de novo, no túnel `novo`, os relays que estavam em outro túnel com o mesmo par.
    pub(super) fn migrate_relays(&mut self, antigos: &[Relay], novo: u32) {
        let Some(h) = self.hosts.get(&novo) else {
            return;
        };
        let novo_vpn = h.vpn;
        let Some(&meu) = self.my_addrs.first() else {
            return;
        };
        for r in antigos {
            let existente = self
                .hosts
                .get(&novo)
                .and_then(|h| h.relay_for.get(&r.peer))
                .copied();
            let indice = match existente {
                Some(e) if e.state != RelayState::Requested => continue,
                Some(e) => e.local_index,
                None => match self.add_relay(novo, r.peer, None, r.kind, RelayState::Requested) {
                    Some(i) => i,
                    None => continue,
                },
            };
            let (from, to) = match r.kind {
                RelayKind::Terminal => (meu, r.peer),
                RelayKind::Forwarding => (r.peer, novo_vpn),
            };
            self.send_relay_request(novo, indice, from, to);
        }
    }

    /// Os relays do túnel `indice` (que deixou de ser o principal) passam para o principal.
    pub(super) fn migrate_to_primary(&mut self, indice: u32) {
        let Some(h) = self.hosts.get(&indice) else {
            return;
        };
        if h.relay_for.is_empty() {
            return;
        }
        let Some(&principal) = self.primary.get(&h.vpn) else {
            return;
        };
        if principal == indice {
            return;
        }
        let antigos: Vec<Relay> = h.relay_for.values().copied().collect();
        self.migrate_relays(&antigos, principal);
    }
}
