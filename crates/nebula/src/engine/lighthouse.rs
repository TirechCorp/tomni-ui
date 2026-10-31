//! O lighthouse: o cliente (perguntar onde os pares estão, avisar onde estamos, furar o NAT) e
//! o servidor (guardar os endereços dos nós e contá-los a quem pergunta).
//!
//! Portado do Nebula (github.com/slackhq/nebula, licença MIT, `lighthouse.go`). O servidor existe
//! para os testes do motor (e, no futuro, para uma rede só de apps), mas o cliente é o que o app
//! usa.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::time::Instant;

use super::{Engine, Key};
use crate::cert::Version;
use crate::header::MessageType;
use crate::meta::{Details, Meta, MetaType};
use crate::underlay::Underlay;

/// Onde apontar o "socket conectado" para descobrir o IP local quando ainda não há lighthouse
/// com endereço: um endereço da faixa de documentação (RFC 5737), que nunca recebe nada.
const PROBE: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 9);

/// O primeiro atraso para tentar de novo um lighthouse que não respondeu.
const RETRY_START: Duration = Duration::from_secs(2);

impl<U: Underlay> Engine<U> {
    /// A versão do formato das mensagens com `vpn`: a do certificado que usamos no túnel, ou a
    /// do config se ainda não há túnel.
    fn meta_version(&self, vpn: IpAddr) -> Version {
        self.primary
            .get(&vpn)
            .and_then(|i| self.hosts.get(i))
            .map_or(self.creds.initiating, |h| h.my_cert_version)
    }

    /// Os detalhes com o endereço `vpn` no campo da versão.
    fn details_for(vpn: IpAddr, versao: Version) -> Option<Details> {
        match (versao, vpn) {
            (Version::V1, IpAddr::V4(v4)) => Some(Details {
                old_vpn_addr: u32::from(v4),
                ..Details::default()
            }),
            (Version::V1, IpAddr::V6(_)) => None,
            (Version::V2, _) => Some(Details {
                vpn_addr: Some(vpn),
                ..Details::default()
            }),
        }
    }

    /// Pergunta a todos os lighthouses onde `vpn` está.
    pub(super) fn query_lighthouses(&mut self, vpn: IpAddr) {
        if self.am_lighthouse || self.lighthouses.contains(&vpn) {
            return;
        }
        let lista: Vec<IpAddr> = self.cfg.lighthouse.hosts.clone();
        for lh in lista {
            let versao = self.meta_version(lh);
            let Some(detalhes) = Self::details_for(vpn, versao) else {
                continue;
            };
            let carga = Meta {
                kind: MetaType::HostQuery,
                details: Some(detalhes),
            }
            .marshal();
            self.send_message_to_vpn(lh, MessageType::LightHouse, 0, carga);
        }
    }

    /// Os endereços UDP que este nó anuncia.
    fn advertised(&self) -> Vec<SocketAddr> {
        let mut lista = if self.advertise.is_empty() {
            let alvo = self
                .cfg
                .lighthouse
                .hosts
                .iter()
                .filter_map(|lh| self.primary.get(lh).and_then(|i| self.hosts.get(i)))
                .find_map(|h| h.remote)
                .unwrap_or(PROBE);
            self.under
                .local_ip_toward(alvo)
                .map(|ip| vec![SocketAddr::new(ip, self.local_port)])
                .unwrap_or_default()
        } else {
            self.advertise.clone()
        };
        lista.retain(|a| a.is_ipv4() && !self.in_my_networks(a.ip()));
        lista
    }

    /// Põe os relays nos detalhes, no formato da versão (o v1 só leva IPv4).
    fn put_relays(detalhes: &mut Details, versao: Version, relays: &[IpAddr]) {
        if versao == Version::V1 {
            detalhes.old_relays = relays
                .iter()
                .filter_map(|r| match r {
                    IpAddr::V4(v4) => Some(u32::from(*v4)),
                    IpAddr::V6(_) => None,
                })
                .collect();
        } else {
            detalhes.relays = relays.to_vec();
        }
    }

    /// Avisa um lighthouse dos nossos endereços e dos relays pelos quais aceitamos ser
    /// alcançados.
    fn send_update(&mut self, lh: IpAddr) {
        let versao = self.meta_version(lh);
        let mut detalhes = Details {
            addrs: self.advertised(),
            ..Details::default()
        };
        Self::put_relays(&mut detalhes, versao, &self.relays_for_me());
        if versao == Version::V1
            && let Some(IpAddr::V4(meu)) = self.my_addrs.iter().find(|a| a.is_ipv4()).copied()
        {
            detalhes.old_vpn_addr = u32::from(meu);
        }
        let carga = Meta {
            kind: MetaType::HostUpdateNotification,
            details: Some(detalhes),
        }
        .marshal();
        self.send_message_to_vpn(lh, MessageType::LightHouse, 0, carga);
    }

    /// O temporizador do aviso periódico.
    pub(super) fn lighthouse_update_tick(&mut self) {
        // Os túneis com os nossos relays ficam de pé, como no Nebula.
        self.connect_relays();
        let lista: Vec<IpAddr> = self.cfg.lighthouse.hosts.clone();
        for lh in lista {
            if self.primary.contains_key(&lh) {
                self.send_update(lh);
            } else {
                // O túnel caiu: abre de novo.
                self.start_handshake(lh);
            }
        }
        let intervalo = self.cfg.lighthouse.interval;
        self.timers
            .add(Instant::now() + intervalo, Key::LighthouseUpdate);
    }

    /// O túnel com um lighthouse abriu: avisa os endereços já, sem esperar o intervalo.
    pub(super) fn lighthouse_up(&mut self, lh: IpAddr) {
        self.lighthouse_errors.remove(&lh);
        self.lighthouse_backoff.remove(&lh);
        if !self.am_lighthouse {
            self.send_update(lh);
        }
    }

    /// O aperto de mão com um lighthouse esgotou as tentativas: tenta de novo, esperando cada
    /// vez mais (até o intervalo do aviso).
    pub(super) fn lighthouse_failed(&mut self, lh: IpAddr, motivo: String) {
        self.lighthouse_errors.insert(lh, motivo);
        let teto = self.cfg.lighthouse.interval.max(RETRY_START);
        let espera = *self.lighthouse_backoff.get(&lh).unwrap_or(&RETRY_START);
        self.lighthouse_backoff.insert(lh, (espera * 2).min(teto));
        self.timers
            .add(Instant::now() + espera, Key::LighthouseRetry(lh));
        self.recompute_state();
    }

    pub(super) fn lighthouse_retry(&mut self, lh: IpAddr) {
        if !self.primary.contains_key(&lh) {
            self.start_handshake(lh);
        }
    }

    /// Uma mensagem do lighthouse chegou (do servidor para o cliente ou o contrário).
    pub(super) fn on_meta(&mut self, indice: u32, carga: &[u8]) {
        let Some(meta) = Meta::unmarshal(carga) else {
            return;
        };
        let Some(detalhes) = meta.details else {
            return;
        };
        let Some(host) = self.hosts.get(&indice) else {
            return;
        };
        let remetente = host.vpn;
        let eh_lighthouse = self.lighthouses.contains(&remetente);
        match meta.kind {
            MetaType::HostQuery if self.am_lighthouse => {
                self.serve_query(indice, remetente, &detalhes);
            }
            MetaType::HostUpdateNotification if self.am_lighthouse => {
                self.serve_update(indice, remetente, detalhes);
            }
            MetaType::HostQueryReply if eh_lighthouse => {
                let Some((vpn, _)) = detalhes.vpn_addr_and_version() else {
                    return;
                };
                if self.my_addrs.contains(&vpn) {
                    return;
                }
                let relays = detalhes.all_relays();
                let lista = self.remotes.entry(vpn).or_default();
                let mudou = lista.set_reported(remetente, detalhes.addrs);
                let relays_mudaram = lista.set_relays(remetente, relays);
                if mudou {
                    self.retrigger(vpn);
                }
                if relays_mudaram && self.pending.contains_key(&vpn) {
                    self.start_relays(vpn);
                }
            }
            MetaType::HostPunchNotification if eh_lighthouse => {
                let Some((vpn, _)) = detalhes.vpn_addr_and_version() else {
                    return;
                };
                let agora = Instant::now();
                if self.cfg.punchy.punch {
                    for alvo in detalhes.addrs.iter().filter(|a| a.is_ipv4()) {
                        self.timers
                            .add(agora + self.cfg.punchy.delay, Key::Punch(*alvo));
                    }
                }
                if self.cfg.punchy.respond {
                    self.timers
                        .add(agora + self.cfg.punchy.respond_delay, Key::Respond(vpn));
                }
            }
            _ => {}
        }
    }

    /// O lighthouse conta onde está o nó perguntado e avisa esse nó para furar o NAT.
    fn serve_query(&mut self, indice: u32, quem_pergunta: IpAddr, detalhes: &Details) {
        let Some((procurado, versao)) = detalhes.vpn_addr_and_version() else {
            return;
        };
        let (conhecidos, relays) = self
            .remotes
            .get(&procurado)
            .map(|l| (l.known(&self.my_nets), l.relays()))
            .unwrap_or_default();
        if conhecidos.is_empty() && relays.is_empty() {
            return;
        }
        let Some(mut resposta) = Self::details_for(procurado, versao) else {
            return;
        };
        Self::put_relays(&mut resposta, versao, &relays);
        resposta.addrs = self.fit_addrs(MetaType::HostQueryReply, resposta.clone(), conhecidos);
        let carga = Meta {
            kind: MetaType::HostQueryReply,
            details: Some(resposta),
        }
        .marshal();
        self.send_to_host(indice, MessageType::LightHouse, 0, &carga);

        // Avisa o procurado de quem está atrás dele.
        let enderecos_de_quem_pergunta = self
            .remotes
            .get(&quem_pergunta)
            .map(|l| l.known(&self.my_nets))
            .unwrap_or_default();
        let Some(&alvo) = self.primary.get(&procurado) else {
            return;
        };
        if enderecos_de_quem_pergunta.is_empty() {
            return;
        }
        let versao_do_alvo = self.meta_version(procurado);
        let Some(mut aviso) = Self::details_for(quem_pergunta, versao_do_alvo) else {
            return;
        };
        aviso.addrs = self.fit_addrs(
            MetaType::HostPunchNotification,
            aviso.clone(),
            enderecos_de_quem_pergunta,
        );
        let carga = Meta {
            kind: MetaType::HostPunchNotification,
            details: Some(aviso),
        }
        .marshal();
        self.send_to_host(alvo, MessageType::LightHouse, 0, &carga);
    }

    /// O lighthouse guarda os endereços que o nó contou e confirma.
    fn serve_update(&mut self, indice: u32, remetente: IpAddr, detalhes: Details) {
        if let Some((declarado, _)) = detalhes.vpn_addr_and_version() {
            let confere = self
                .hosts
                .get(&indice)
                .is_some_and(|h| h.addrs.contains(&declarado));
            if !confere {
                return;
            }
        }
        let versao = detalhes.vpn_addr_and_version().map(|(_, v)| v);
        let relays = detalhes.all_relays();
        let lista = self.remotes.entry(remetente).or_default();
        lista.set_reported(remetente, detalhes.addrs);
        lista.set_relays(remetente, relays);
        let versao = versao.unwrap_or_else(|| self.meta_version(remetente));
        let mut ack = Details::default();
        if versao == Version::V1
            && let Some(IpAddr::V4(meu)) = self.my_addrs.iter().find(|a| a.is_ipv4()).copied()
        {
            ack.old_vpn_addr = u32::from(meu);
        }
        let carga = Meta {
            kind: MetaType::HostUpdateNotificationAck,
            details: Some(ack),
        }
        .marshal();
        self.send_to_host(indice, MessageType::LightHouse, 0, &carga);
    }

    /// Corta a lista de endereços para a mensagem caber no MTU do túnel (menos o cabeçalho e a
    /// etiqueta). O Nebula limita a lista por um número fixo; aqui o que manda é o tamanho.
    fn fit_addrs(
        &self,
        tipo: MetaType,
        base: Details,
        mut enderecos: Vec<SocketAddr>,
    ) -> Vec<SocketAddr> {
        let limite =
            usize::from(self.cfg.mtu).saturating_sub(crate::header::LEN + crate::noise::TAG_LEN);
        while !enderecos.is_empty() {
            let teste = Meta {
                kind: tipo,
                details: Some(Details {
                    addrs: enderecos.clone(),
                    ..base.clone()
                }),
            };
            if teste.marshal().len() <= limite {
                break;
            }
            enderecos.pop();
        }
        enderecos
    }
}
