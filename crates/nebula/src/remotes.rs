//! Os endereços UDP conhecidos de cada par: a lista de remotos do Nebula.
//!
//! Portado do Nebula (github.com/slackhq/nebula, licença MIT, `remote_list.go`). Cada par tem os
//! endereços do `static_host_map`, o que cada lighthouse **contou** dele (`reported`), o último
//! de onde um pacote autêntico dele chegou (`learned`) e os que já se mostraram errados
//! (`blocked`). Guarda também os relays pelos quais cada dono disse que o par aceita ser
//! alcançado.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};

use tomni_ui_netstack::Cidr;

/// Os endereços de um par.
#[derive(Default, Debug)]
pub(crate) struct RemoteList {
    /// Os do `static_host_map`, já resolvidos.
    pub(crate) statics: Vec<SocketAddr>,
    /// Ainda falta resolver algum nome de `statics`: o aperto de mão espera.
    pub(crate) resolving: bool,
    /// O que cada dono (um lighthouse, ou o próprio par) contou.
    reported: HashMap<IpAddr, Vec<SocketAddr>>,
    /// De onde chegou o último pacote autêntico do par.
    learned: Option<SocketAddr>,
    /// Os que responderam com outro nó: não se tenta mais.
    blocked: HashSet<SocketAddr>,
    /// Os relays do par (IPs da VPN), por quem contou.
    relays: HashMap<IpAddr, Vec<IpAddr>>,
}

/// Só o IPv4 vale: o underlay não tem IPv6.
fn usable(addr: &SocketAddr) -> bool {
    addr.is_ipv4() && addr.port() != 0 && !addr.ip().is_unspecified()
}

/// Um endereço é privado se não dá para chegar a ele da internet.
fn is_private(addr: &SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(_) => false,
    }
}

impl RemoteList {
    /// Troca o que `owner` contou. Devolve `true` se mudou.
    pub(crate) fn set_reported(&mut self, owner: IpAddr, addrs: Vec<SocketAddr>) -> bool {
        let addrs: Vec<SocketAddr> = addrs.into_iter().filter(usable).collect();
        match self.reported.get(&owner) {
            Some(atual) if *atual == addrs => false,
            _ => {
                self.reported.insert(owner, addrs);
                true
            }
        }
    }

    /// Troca os relays que `owner` contou. Devolve `true` se mudou.
    pub(crate) fn set_relays(&mut self, owner: IpAddr, relays: Vec<IpAddr>) -> bool {
        match self.relays.get(&owner) {
            Some(atual) if *atual == relays => false,
            None if relays.is_empty() => false,
            _ => {
                self.relays.insert(owner, relays);
                true
            }
        }
    }

    /// Os relays do par, sem repetir, na ordem de quem contou.
    pub(crate) fn relays(&self) -> Vec<IpAddr> {
        let mut donos: Vec<&IpAddr> = self.relays.keys().collect();
        donos.sort();
        let mut todos = Vec::new();
        for dono in donos {
            for relay in &self.relays[dono] {
                if !todos.contains(relay) {
                    todos.push(*relay);
                }
            }
        }
        todos
    }

    /// Anota de onde chegou um pacote autêntico. Devolve `true` se mudou.
    pub(crate) fn learn(&mut self, addr: SocketAddr) -> bool {
        if !usable(&addr) || self.learned == Some(addr) {
            return false;
        }
        self.learned = Some(addr);
        true
    }

    /// Não tenta mais este endereço.
    pub(crate) fn block(&mut self, addr: SocketAddr) {
        self.blocked.insert(addr);
    }

    /// Esquece os bloqueios (o aperto de mão deu certo).
    pub(crate) fn unblock_all(&mut self) {
        self.blocked.clear();
    }

    /// Os endereços para tentar o aperto de mão, na ordem do Nebula: os públicos antes dos
    /// privados e, dentro de cada grupo, a ordem do endereço. Fora os bloqueados e os que
    /// ficam dentro da rede da VPN (seria o túnel passando por dentro do túnel).
    pub(crate) fn candidates(&self, vpn_networks: &[Cidr]) -> Vec<SocketAddr> {
        let mut todos: Vec<SocketAddr> = self
            .statics
            .iter()
            .chain(self.learned.iter())
            .chain(self.reported.values().flatten())
            .copied()
            .filter(|a| usable(a) && !self.blocked.contains(a))
            .filter(|a| !vpn_networks.iter().any(|rede| rede.contains(a.ip())))
            .collect();
        todos.sort_by_key(|a| (is_private(a), *a));
        todos.dedup();
        todos
    }

    /// O que um lighthouse conta a quem pergunta: o que o par disse de si e de onde o
    /// lighthouse o viu chegar.
    pub(crate) fn known(&self, vpn_networks: &[Cidr]) -> Vec<SocketAddr> {
        let mut todos: Vec<SocketAddr> = self
            .learned
            .iter()
            .chain(self.reported.values().flatten())
            .copied()
            .filter(|a| usable(a) && !vpn_networks.iter().any(|rede| rede.contains(a.ip())))
            .collect();
        todos.sort_by_key(|a| (is_private(a), *a));
        todos.dedup();
        todos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(texto: &str) -> SocketAddr {
        texto.parse().unwrap()
    }

    #[test]
    fn os_publicos_vem_antes_dos_privados_e_ipv6_fica_de_fora() {
        let mut lista = RemoteList {
            statics: vec![a("192.168.1.5:4242"), a("203.0.113.7:4242")],
            ..RemoteList::default()
        };
        lista.set_reported(
            "10.42.0.1".parse().unwrap(),
            vec![a("[2001:db8::1]:4242"), a("198.51.100.2:5000")],
        );
        assert_eq!(
            lista.candidates(&[]),
            [
                a("198.51.100.2:5000"),
                a("203.0.113.7:4242"),
                a("192.168.1.5:4242")
            ]
        );
    }

    #[test]
    fn bloqueados_e_enderecos_da_vpn_saem() {
        let rede = Cidr::new("10.42.0.0".parse().unwrap(), 24).unwrap();
        let mut lista = RemoteList {
            statics: vec![a("203.0.113.7:4242"), a("10.42.0.9:4242")],
            ..RemoteList::default()
        };
        lista.block(a("203.0.113.7:4242"));
        assert!(lista.candidates(&[rede]).is_empty());
        lista.unblock_all();
        assert_eq!(lista.candidates(&[rede]), [a("203.0.113.7:4242")]);
    }

    #[test]
    fn learn_e_set_reported_dizem_se_mudou() {
        let mut lista = RemoteList::default();
        assert!(lista.learn(a("203.0.113.7:1")));
        assert!(!lista.learn(a("203.0.113.7:1")));
        let dono: IpAddr = "10.42.0.1".parse().unwrap();
        assert!(lista.set_reported(dono, vec![a("203.0.113.8:1")]));
        assert!(!lista.set_reported(dono, vec![a("203.0.113.8:1")]));
        assert!(lista.set_reported(dono, vec![]));
        assert_eq!(lista.known(&[]), [a("203.0.113.7:1")]);
    }

    #[test]
    fn os_relays_juntam_o_que_cada_dono_contou_sem_repetir() {
        let mut lista = RemoteList::default();
        let lh1: IpAddr = "10.42.0.1".parse().unwrap();
        let lh2: IpAddr = "10.42.0.2".parse().unwrap();
        let r1: IpAddr = "10.42.0.7".parse().unwrap();
        let r2: IpAddr = "10.42.0.8".parse().unwrap();
        assert!(!lista.set_relays(lh1, Vec::new()));
        assert!(lista.set_relays(lh1, vec![r1]));
        assert!(!lista.set_relays(lh1, vec![r1]));
        assert!(lista.set_relays(lh2, vec![r1, r2]));
        assert_eq!(lista.relays(), vec![r1, r2]);
        assert!(lista.set_relays(lh1, Vec::new()));
        assert_eq!(lista.relays(), vec![r1, r2]);
    }
}
