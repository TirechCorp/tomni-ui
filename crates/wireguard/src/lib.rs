//! WireGuard em Rust para o Tomni UI.
//!
//! Tudo em Rust, sem `unsafe` e sem FFI. O crate fala o protocolo do WireGuard (handshake
//! Noise IKpsk2, transporte, cookies e temporizadores) e **não abre interface de rede no
//! sistema**: nem placa virtual, nem administrador. Os pacotes IP do túnel passam por uma
//! pilha TCP/IP em software (`tomni-ui-netstack`), e o único acesso ao túnel é o [`proxy`]
//! (um SOCKS5 em `127.0.0.1` com porta e credencial aleatórias): nenhum outro programa do
//! computador enxerga a rede.
//!
//! # Um túnel em poucas linhas
//!
//! ```no_run
//! # async fn exemplo() -> std::io::Result<()> {
//! use std::time::Duration;
//! use tomni_ui_wireguard::{WgConfig, WireGuardBuilder};
//!
//! let texto = std::fs::read_to_string("cliente.conf")?;
//! let (config, avisos) = WgConfig::from_conf(&texto).map_err(std::io::Error::other)?;
//! for aviso in &avisos {
//!     eprintln!("{aviso}");
//! }
//! let wg = WireGuardBuilder::new(config).start().await?;
//! wg.wait_connected(Duration::from_secs(30)).await?;
//! // Os clientes (SSH, SFTP, Telnet...) usam o proxy; a rede não aparece para mais ninguém.
//! let _proxy = wg.proxy();
//! wg.stop(Duration::from_secs(2)).await;
//! # Ok(())
//! # }
//! ```
//!
//! # As peças
//!
//! - [`WgConfig`]: o `.conf` do `wg-quick` lido, com avisos para o que o app não usa
//!   (`Table`, `PostUp`, `ListenPort`...), e [`WgConfig::without_secrets`] para mostrar na tela.
//! - [`WireGuardBuilder`] e [`WireGuard`]: o motor (uma tarefa no runtime do `tokio`), com o
//!   [`State`] e a lista de pares ([`PeerInfo`]).
//! - [`Device`]: vários pares sobre um UDP, sem E/S (o motor é um laço em volta dele).
//! - [`PrivateKey`], [`PublicKey`] e [`PresharedKey`]: as chaves.
//! - [`Peer`]: o estado de um par (handshake, chaves de sessão, temporizadores), também sem
//!   E/S, com o relógio injetado.
//! - [`Router`]: a tabela de `AllowedIPs` (o roteamento por chave criptográfica).
//!
//! [`proxy`]: WireGuard::proxy

mod conf;
mod crypto;
mod device;
mod engine;
mod handshake;
mod indices;
mod key;
mod peer;
mod replay;
mod responder;
mod router;
mod timers;
mod transport;

pub use conf::{
    Aviso, ConfError, Endpoint, InterfaceAddress, InterfaceConfig, PeerSection, WgConfig,
};
pub use device::{Device, DeviceError, Event, SendError};
pub use engine::{PeerInfo, State, WireGuard, WireGuardBuilder};
pub use handshake::{Identity, TAMANHO_COOKIE, TAMANHO_INICIACAO, TAMANHO_RESPOSTA};
pub use indices::{IndexGuard, Indices};
pub use key::{KEY_LEN, KeyError, PresharedKey, PrivateKey, PublicKey};
pub use peer::{AcceptError, MTU_PADRAO, Output, Peer, PeerConfig, PeerError, PeerStats};
pub use responder::{Inbound, OpenedInitiation, Responder};
pub use router::{Cidr, CidrError, Router};
pub use timers::{
    KEEPALIVE_TIMEOUT, REJECT_AFTER_MESSAGES, REJECT_AFTER_TIME, REKEY_AFTER_MESSAGES,
    REKEY_AFTER_TIME, REKEY_ATTEMPT_TIME, REKEY_TIMEOUT,
};
pub use transport::TAMANHO_KEEPALIVE;
