//! Cliente OpenVPN embutido do Tomni UI.
//!
//! Implementa o protocolo do OpenVPN a partir da descrição pública do protocolo (OpenVPN Wire
//! Protocol). OpenVPN é marca registrada da OpenVPN Inc.; este crate não é afiliado.
//!
//! O crate não cria dispositivo TUN nem chama o binário `openvpn`: o canal de controle (TLS) e o
//! de dados falam por UDP ou TCP, e o tráfego do app entra pela pilha em software. Tudo fica
//! dentro do processo; nada aparece para outros programas.
//!
//! O crate tem o `.ovpn` ([`config`]), o PKI ([`pki`], [`static_key`]), as peças puras do
//! protocolo ([`packet`], [`wrap`], [`reliable`], [`replay`], [`prf`], [`keymethod`], [`push`]),
//! o canal de controle ([`control`], sobre o [`channel`]: o TLS com `rustls`, a conferência do
//! certificado ([`verify`]), a senha e os desafios ([`auth`])), o canal de dados ([`data`]: AES-GCM
//! e ChaCha20-Poly1305), a sessão completa sem E/S ([`session`]), o transporte UDP e TCP
//! ([`transport`]) e o motor ([`vpn`]), que liga tudo à pilha TCP/IP em software do
//! `tomni-ui-netstack`.
//!
//! O núcleo é sem E/S: quem usa entrega os datagramas e o relógio (um `Instant`) e retira o que
//! enviar. Com a feature `test-util`, o `test_server` é o lado servidor do protocolo, em
//! memória, para os testes.
//!
//! # Uso
//!
//! O único acesso à rede da VPN é o [`OpenVpn::proxy`]: um SOCKS5 em `127.0.0.1` com porta e
//! credencial aleatórias, que só os clientes do próprio app usam. Mostre-o na interface nunca e
//! não o exponha a outros programas.
//!
//! ```no_run
//! use std::time::Duration;
//! use tomni_ui_openvpn::OpenVpnBuilder;
//! use tomni_ui_openvpn::config::Config;
//!
//! # fn exemplo() -> Result<(), Box<dyn std::error::Error>> {
//! let texto = std::fs::read_to_string("casa.ovpn")?;
//! let (config, _avisos) = Config::from_ovpn(&texto, Some(std::path::Path::new(".")))?;
//! let vpn = OpenVpnBuilder::new(config)
//!     .on_state(|estado| eprintln!("{estado:?}"))
//!     .start()?;
//! vpn.wait_connected(Duration::from_secs(30))?;
//! // Passe `vpn.proxy()` ao `.proxy(..)` do `Ssh::builder`, do `Telnet::builder` ou do `Sftp`.
//! let _proxy = vpn.proxy();
//! vpn.stop(Duration::from_secs(2));
//! # Ok(())
//! # }
//! ```

pub mod auth;
pub mod channel;
pub mod config;
pub mod control;
pub mod data;
mod engine;
pub mod keymethod;
pub mod packet;
pub mod pki;
pub mod prf;
pub mod push;
pub mod reliable;
pub mod replay;
pub mod session;
pub mod static_key;
pub mod transport;
pub mod verify;
pub mod vpn;
pub mod wrap;

pub use vpn::{Error, OpenVpn, OpenVpnBuilder, Prompt, PromptKind, State};

#[cfg(feature = "test-util")]
pub mod test_server;
