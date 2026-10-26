//! O ZeroTier do Tomni UI em Rust (M61): um nó de uma rede virtual do ZeroTier dentro do app,
//! sem `zerotier-one`, sem placa virtual, sem administrador, sem FFI e sem C++.
//!
//! O crate junta a identidade do nó ([`identity`]), a camada de pares do protocolo (VL1: cifra,
//! pacotes, raízes, caminhos) e a das redes virtuais (VL2: configuração assinada, membros,
//! regras, ARP e multicast). A pilha TCP/IP em software e o proxy SOCKS5 vêm do crate
//! `tomni-ui-netstack`.
//!
//! **Nada é exposto ao computador.** O único acesso à rede é o proxy SOCKS5 em `127.0.0.1`
//! com porta e credencial aleatórias, que só os clientes do app usam.

pub mod address;
pub mod crypto;
pub mod dictionary;
pub mod engine;
pub mod identity;
pub mod inet;
pub mod netconf;
pub mod node;
pub mod packet;
pub mod path;
pub mod peer;
pub mod vl1;
pub mod vl2;
pub mod world;

#[cfg(feature = "test-util")]
pub mod testing;

pub use address::{ADDRESS_LEN, Address, NetworkId};
pub use identity::Identity;
pub use node::{NodeState, PeerInfo, State, ZeroTier, ZeroTierBuilder};
pub use tomni_ui_netstack::Cidr;
pub use world::{World, WorldKind};

use std::fmt;

/// O erro das peças do crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A identidade não está no formato do ZeroTier ou não passa na validação.
    Identity(String),
    /// O pacote está mal formado (curto demais, campo fora do lugar, compressão inválida).
    Packet(String),
    /// A autenticação do pacote falhou: chave errada, pacote adulterado ou repetido.
    Authentication,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Identity(motivo) => write!(f, "identidade inválida: {motivo}"),
            Error::Packet(motivo) => write!(f, "pacote inválido: {motivo}"),
            Error::Authentication => write!(f, "a autenticação do pacote falhou"),
        }
    }
}

impl std::error::Error for Error {}

/// O resultado das peças do crate.
pub type Result<T> = std::result::Result<T, Error>;
