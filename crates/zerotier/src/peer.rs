//! Um par conhecido: a identidade, a chave que os dois compartilham, os caminhos físicos, a
//! versão que ele anunciou e se ele pode falar conosco sem que a gente tenha procurado.

use std::fmt;
use std::net::SocketAddr;

use crate::crypto::PeerKey;
use crate::path::Path;
use crate::{Address, Identity};

/// A versão que o outro lado anunciou no HELLO ou no OK(HELLO).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteVersion {
    /// A versão do protocolo (a que decide a cifra, seção 2.1 do plano).
    pub protocol: u8,
    /// A versão do programa: maior.
    pub major: u8,
    /// A versão do programa: menor.
    pub minor: u8,
    /// A versão do programa: revisão.
    pub revision: u16,
}

impl fmt::Display for RemoteVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}.{} (protocolo {})",
            self.major, self.minor, self.revision, self.protocol
        )
    }
}

/// O papel de um par para o nosso nó.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerRole {
    /// Raiz do planet.
    Root,
    /// Raiz de uma moon que orbitamos.
    Moon,
    /// O controlador da nossa rede.
    Controller,
    /// Outro membro (ou qualquer par).
    Member,
}

/// Um par.
pub struct Peer {
    identity: Identity,
    key: PeerKey,
    paths: Vec<Path>,
    version: Option<RemoteVersion>,
    last_receive: u64,
    last_full_hello: u64,
    contacted: bool,
}

impl Peer {
    /// Um par novo. `None` se o acordo de chaves com a nossa identidade falhar (a chave pública
    /// dele não vale) ou se a nossa não tem a parte secreta.
    pub fn new(ours: &Identity, identity: Identity, now: u64) -> Option<Self> {
        let acordo = ours.agree(&identity)?;
        Some(Self {
            key: PeerKey::from_agreement(&acordo),
            identity,
            paths: Vec::new(),
            version: None,
            last_receive: now,
            last_full_hello: 0,
            contacted: false,
        })
    }

    /// O endereço do par.
    pub fn address(&self) -> Address {
        self.identity.address()
    }

    /// A identidade.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// A chave compartilhada.
    pub fn key(&self) -> &PeerKey {
        &self.key
    }

    /// A versão anunciada, se já se apresentou.
    pub fn version(&self) -> Option<RemoteVersion> {
        self.version
    }

    /// Guarda a versão anunciada.
    pub fn set_version(&mut self, versao: RemoteVersion) {
        self.version = Some(versao);
    }

    /// O último instante em que chegou algo autenticado dele.
    pub fn last_receive(&self) -> u64 {
        self.last_receive
    }

    /// Chegou algo autenticado do par, por `de` se for direto.
    pub fn received(&mut self, now: u64) {
        self.last_receive = now;
    }

    /// Procuramos este par (mandamos algo a ele) ou um raiz nos apresentou: ele pode falar
    /// conosco e receber respostas.
    pub fn contacted(&self) -> bool {
        self.contacted
    }

    /// Marca que o par é conhecido por iniciativa nossa.
    pub fn set_contacted(&mut self) {
        self.contacted = true;
    }

    /// Os caminhos conhecidos.
    pub fn paths(&self) -> &[Path] {
        &self.paths
    }

    /// Os caminhos conhecidos, para mudar.
    pub fn paths_mut(&mut self) -> &mut Vec<Path> {
        &mut self.paths
    }

    /// O caminho com este endereço.
    pub fn path_mut(&mut self, addr: SocketAddr) -> Option<&mut Path> {
        self.paths.iter_mut().find(|c| c.addr() == addr)
    }

    /// Aprende o caminho, ou renova o que já existe. Devolve se era novo.
    pub fn learn_path(&mut self, addr: SocketAddr, now: u64) -> bool {
        if let Some(c) = self.path_mut(addr) {
            c.received(now);
            return false;
        }
        self.paths.push(Path::new(addr, now));
        true
    }

    /// O melhor caminho vivo.
    pub fn best_path(&self, now: u64) -> Option<&Path> {
        self.paths
            .iter()
            .filter(|c| c.alive(now))
            .min_by_key(|c| c.score(now))
    }

    /// Há algum caminho vivo.
    pub fn has_alive_path(&self, now: u64) -> bool {
        self.paths.iter().any(|c| c.alive(now))
    }

    /// A menor latência entre os caminhos vivos.
    pub fn latency(&self, now: u64) -> Option<u32> {
        self.best_path(now).and_then(Path::latency)
    }

    /// Esquece os caminhos vencidos. Devolve quantos saíram.
    pub fn expire_paths(&mut self, now: u64) -> usize {
        let antes = self.paths.len();
        self.paths.retain(|c| !c.expired(now));
        antes - self.paths.len()
    }

    /// Já passou a hora do HELLO completo (um por minuto, `ZT_PEER_PING_PERIOD`)? Se passou,
    /// registra que vai sair um.
    pub fn take_full_hello(&mut self, now: u64) -> bool {
        if now.saturating_sub(self.last_full_hello) >= crate::vl1::PING_PERIOD {
            self.last_full_hello = now;
            true
        } else {
            false
        }
    }
}

impl fmt::Debug for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Peer")
            .field("address", &self.address())
            .field("paths", &self.paths.len())
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn par() -> Peer {
        let a = Identity::from_secret_str(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/identidades/id1.secret"
            ))
            .unwrap(),
        )
        .unwrap();
        let b = Identity::from_secret_str(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/identidades/id2.secret"
            ))
            .unwrap(),
        )
        .unwrap();
        Peer::new(&a, b, 0).unwrap()
    }

    #[test]
    fn aprende_caminhos_e_escolhe_o_melhor_vivo() {
        let mut p = par();
        let a: SocketAddr = "192.0.2.1:1".parse().unwrap();
        let b: SocketAddr = "192.0.2.2:2".parse().unwrap();
        assert!(p.learn_path(a, 1_000));
        assert!(!p.learn_path(a, 2_000));
        assert!(p.learn_path(b, 2_000));
        p.path_mut(a).unwrap().set_latency(80);
        p.path_mut(b).unwrap().set_latency(20);
        assert_eq!(p.best_path(3_000).unwrap().addr(), b);
        assert_eq!(p.latency(3_000), Some(20));
        assert!(p.best_path(1_000_000).is_none(), "tudo vencido");
    }

    #[test]
    fn os_caminhos_vencidos_saem() {
        let mut p = par();
        p.learn_path("192.0.2.1:1".parse().unwrap(), 0);
        assert_eq!(p.expire_paths(1_000), 0);
        assert_eq!(p.expire_paths(crate::path::PATH_EXPIRATION), 1);
        assert!(p.paths().is_empty());
    }

    #[test]
    fn o_hello_completo_sai_uma_vez_por_minuto() {
        let mut p = par();
        assert!(p.take_full_hello(100_000));
        assert!(!p.take_full_hello(100_000 + crate::vl1::PING_PERIOD - 1));
        assert!(p.take_full_hello(100_000 + crate::vl1::PING_PERIOD));
    }
}
