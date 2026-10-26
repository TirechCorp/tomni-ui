//! O plano de dados: o que chega pelo UDP, o que sai da pilha e a conferência dos túneis.
//!
//! Portado do Nebula (`outside.go`, `inside.go` e `connection_manager.go`).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::time::Instant;

use super::{Cached, Engine, Key, ROAMING_SUPPRESS};
use crate::cert::CertError;
use crate::header::{self, Header, MessageType};
use crate::tunnel::DecryptError;
use crate::underlay::Underlay;
use crate::util::now_unix;

impl<U: Underlay> Engine<U> {
    /// Um datagrama chegou.
    pub(super) fn on_udp(&mut self, pacote: &[u8], de: SocketAddr) {
        // Os pacotes de punch têm um byte só e não têm cabeçalho: caem aqui.
        let Some(h) = Header::parse(pacote) else {
            return;
        };
        if h.version != header::VERSION {
            return;
        }
        // Um pacote "de dentro da rede" vindo do underlay seria o túnel passando por dentro dele.
        if self.in_my_networks(de.ip()) {
            return;
        }
        match h.kind {
            MessageType::Handshake => self.on_handshake(pacote, de, h),
            MessageType::RecvError => self.on_recv_error(de, h),
            MessageType::Message
            | MessageType::LightHouse
            | MessageType::Test
            | MessageType::CloseTunnel => self.on_encrypted(pacote, de, h),
            MessageType::Control => {}
        }
    }

    fn on_encrypted(&mut self, pacote: &[u8], de: SocketAddr, h: Header) {
        // Os pacotes por relay ficam para uma versão futura.
        if h.kind == MessageType::Message && h.subtype == header::MESSAGE_RELAY {
            return;
        }
        let indice = h.remote_index;
        let Some(host) = self.hosts.get_mut(&indice) else {
            if h.kind != MessageType::CloseTunnel {
                self.send_recv_error(de, indice);
            }
            return;
        };
        let carga = match host.tunnel.decrypt(pacote, h.counter) {
            Ok(carga) => carga,
            Err(DecryptError::Replay | DecryptError::Authentication) => return,
        };
        host.in_traffic = true;
        self.roam(indice, de);
        match h.kind {
            MessageType::Message => self.on_ip_packet(indice, carga),
            MessageType::LightHouse => self.on_meta(indice, &carga),
            MessageType::Test => {
                if h.subtype == header::TEST_REQUEST {
                    self.send_to_host(indice, MessageType::Test, header::TEST_REPLY, &carga);
                }
            }
            MessageType::CloseTunnel => self.delete_host(indice),
            _ => {}
        }
    }

    /// "Não conheço esse índice": o cabeçalho de 16 bytes, sem carga.
    fn send_recv_error(&self, para: SocketAddr, indice: u32) {
        let cabecalho = header::encode(MessageType::RecvError, 0, indice, 0);
        self.raw_send(&cabecalho, para);
    }

    /// O par diz que não conhece o túnel: refazemos.
    fn on_recv_error(&mut self, de: SocketAddr, h: Header) {
        let Some(indice) = self
            .hosts
            .iter()
            .find(|(_, host)| host.remote_index == h.remote_index)
            .map(|(indice, _)| *indice)
        else {
            return;
        };
        let Some(host) = self.hosts.get(&indice) else {
            return;
        };
        // Alguém de outro endereço não pode derrubar o túnel (o erro não é autenticado).
        if host.remote.is_some_and(|r| r != de) {
            return;
        }
        let vpn = host.vpn;
        self.delete_host(indice);
        if self.lighthouses.contains(&vpn) {
            self.start_handshake(vpn);
        }
    }

    /// Um pacote autêntico chegou de outro endereço: o par mudou de lugar.
    fn roam(&mut self, indice: u32, de: SocketAddr) {
        let Some(host) = self.hosts.get_mut(&indice) else {
            return;
        };
        if host.remote == Some(de) {
            return;
        }
        let agora = Instant::now();
        if let Some((anterior, quando)) = host.last_roam
            && anterior == de
            && agora.duration_since(quando) < ROAMING_SUPPRESS
        {
            // Voltar ao endereço de que acabamos de sair é, quase sempre, um pacote atrasado.
            return;
        }
        if let Some(antigo) = host.remote {
            host.last_roam = Some((antigo, agora));
        }
        self.set_remote(indice, de);
    }

    /// Um pacote IP decifrado: confere a origem e entrega à pilha.
    fn on_ip_packet(&mut self, indice: u32, carga: Vec<u8>) {
        let Some(host) = self.hosts.get_mut(&indice) else {
            return;
        };
        if carga.len() < 20 || carga[0] >> 4 != 4 {
            return;
        }
        let origem = IpAddr::V4(Ipv4Addr::new(carga[12], carga[13], carga[14], carga[15]));
        let permitida = host.addrs.contains(&origem)
            || host.unsafe_nets.iter().any(|rede| rede.contains(origem));
        if !permitida {
            return;
        }
        host.last_activity = Instant::now();
        // A fila cheia perde o pacote, como uma placa de rede.
        let _ = self.net_tx.try_send(carga);
    }

    /// Um pacote IP saiu da pilha.
    pub(super) fn on_inside(&mut self, pacote: Vec<u8>) {
        if pacote.len() < 20 || pacote[0] >> 4 != 4 {
            return;
        }
        let destino = IpAddr::V4(Ipv4Addr::new(
            pacote[16], pacote[17], pacote[18], pacote[19],
        ));
        if self.my_addrs.contains(&destino) {
            return;
        }
        let Some(alvo) = self.route_to(destino) else {
            self.unreachable(
                destino,
                format!("{destino} não está em nenhuma rede do certificado nem das unsafe_routes"),
            );
            return;
        };
        match self.primary.get(&alvo).copied() {
            Some(indice) => {
                self.send_ip(indice, &pacote);
            }
            None => self.cache_and_handshake(alvo, Cached::Ip(pacote)),
        }
    }

    /// O par da VPN por onde `destino` sai: ele mesmo, se está na rede, ou o gateway de uma
    /// rota `unsafe`.
    fn route_to(&self, destino: IpAddr) -> Option<IpAddr> {
        if self.in_my_networks(destino) {
            return Some(destino);
        }
        let rota = self
            .cfg
            .unsafe_routes
            .iter()
            .filter(|r| r.route.contains(destino))
            .max_by_key(|r| r.route.prefix())?;
        rota.via
            .iter()
            .find(|g| self.primary.contains_key(g))
            .or_else(|| rota.via.first())
            .copied()
    }

    /// Manda um pacote IP por um túnel.
    pub(super) fn send_ip(&mut self, indice: u32, pacote: &[u8]) -> bool {
        let enviado = self.send_to_host(indice, MessageType::Message, 0, pacote);
        if enviado && let Some(host) = self.hosts.get_mut(&indice) {
            host.last_activity = Instant::now();
        }
        enviado
    }

    /// Cifra e manda uma mensagem por um túnel.
    pub(super) fn send_to_host(
        &mut self,
        indice: u32,
        tipo: MessageType,
        subtipo: u8,
        carga: &[u8],
    ) -> bool {
        let Some(host) = self.hosts.get_mut(&indice) else {
            return false;
        };
        let Some(remoto) = host.remote else {
            return false;
        };
        let Some(pacote) = host.tunnel.encrypt(tipo, subtipo, host.remote_index, carga) else {
            return false;
        };
        host.out_traffic = true;
        self.raw_send(&pacote, remoto);
        true
    }

    /// Manda uma mensagem do Nebula a um par pelo endereço da VPN. Sem túnel, guarda e abre.
    pub(super) fn send_message_to_vpn(
        &mut self,
        vpn: IpAddr,
        tipo: MessageType,
        subtipo: u8,
        carga: Vec<u8>,
    ) {
        match self.primary.get(&vpn).copied() {
            Some(indice) => {
                self.send_to_host(indice, tipo, subtipo, &carga);
            }
            None => self.cache_and_handshake(
                vpn,
                Cached::Message {
                    kind: tipo,
                    subtype: subtipo,
                    payload: carga,
                },
            ),
        }
    }

    /// O punch: um byte para manter o NAT aberto. Lighthouses não precisam.
    pub(super) fn send_punch(&self, indice: u32) {
        if !self.cfg.punchy.punch {
            return;
        }
        let Some(host) = self.hosts.get(&indice) else {
            return;
        };
        if self.lighthouses.contains(&host.vpn) {
            return;
        }
        if let Some(remoto) = host.remote {
            self.raw_send(&[1], remoto);
        }
    }

    fn send_punch_to_all(&self, indice: u32) {
        if !self.cfg.punchy.punch {
            return;
        }
        let Some(host) = self.hosts.get(&indice) else {
            return;
        };
        for destino in self.candidates(host.vpn) {
            self.raw_send(&[1], destino);
        }
    }

    fn send_test(&mut self, indice: u32) {
        self.send_to_host(indice, MessageType::Test, header::TEST_REQUEST, &[]);
    }

    /// Responde a um aviso de punch com um teste, o que abre o túnel se ele não existe.
    pub(super) fn respond_to_punch(&mut self, vpn: IpAddr) {
        self.send_message_to_vpn(vpn, MessageType::Test, header::TEST_REQUEST, Vec::new());
    }

    /// A conferência periódica de um túnel (o `connection_manager` do Nebula).
    pub(super) fn traffic_tick(&mut self, indice: u32) {
        let intervalo = self.cfg.connection_alive_interval;
        let espera_apagar = self.cfg.pending_deletion_interval;
        let agora = Instant::now();
        let Some(host) = self.hosts.get_mut(&indice) else {
            return;
        };
        let vpn = host.vpn;

        // O certificado do par ainda vale?
        if let Err(erro) = self.creds.pool.verify(&host.peer_cert, now_unix()) {
            let derrubar = match erro {
                CertError::Blocklisted { .. } => true,
                CertError::Expired { .. } | CertError::RootExpired { .. } => {
                    self.cfg.pki.disconnect_invalid
                }
                _ => false,
            };
            if derrubar {
                self.close_tunnel(indice);
                return;
            }
        }
        if host.tunnel.exhausted() {
            self.delete_host(indice);
            return;
        }
        let entrou = std::mem::take(&mut host.in_traffic);
        let saiu = std::mem::take(&mut host.out_traffic);
        let principal = self.primary.get(&vpn) == Some(&indice);

        if entrou {
            host.pending_deletion = false;
            let precisa_refazer = principal
                && !host.rehandshaked
                && (host.tunnel.needs_rehandshake()
                    || (host.peer_cert.version() > host.my_cert_version
                        && self.creds.cert(host.peer_cert.version()).is_some()));
            if precisa_refazer {
                host.rehandshaked = true;
            }
            self.timers.add(agora + intervalo, Key::Traffic(indice));
            if precisa_refazer {
                self.start_handshake(vpn);
            }
            if !saiu {
                self.send_punch(indice);
            }
            return;
        }
        // Nada entrou nesse intervalo.
        if host.pending_deletion {
            self.delete_host(indice);
            return;
        }
        if principal && !saiu {
            let parado = agora.duration_since(host.last_activity);
            if self.cfg.drop_inactive && parado >= self.cfg.inactivity_timeout {
                self.close_tunnel(indice);
                return;
            }
            // Ninguém falou: só mantém o NAT aberto e confere de novo depois.
            self.send_punch(indice);
            self.timers.add(agora + intervalo, Key::Traffic(indice));
            return;
        }
        if saiu {
            if self.cfg.punchy.target_all_remotes {
                self.send_punch_to_all(indice);
            }
            self.send_test(indice);
        }
        if let Some(host) = self.hosts.get_mut(&indice) {
            host.pending_deletion = true;
        }
        self.timers.add(agora + espera_apagar, Key::Traffic(indice));
    }
}
