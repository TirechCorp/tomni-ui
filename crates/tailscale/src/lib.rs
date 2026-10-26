//! O Tailscale do Tomni UI em Rust (M57): um nó da tailnet (Tailscale ou Headscale) dentro do
//! app, sem `tailscaled`, sem placa virtual, sem administrador, sem FFI e sem Go.
//!
//! O crate junta: o cliente do plano de controle pelo ts2021 ([`control`]: chave do servidor,
//! canal Noise, registro com chave de autenticação ou URL de login, e o mapa da rede em
//! streaming), o mapa já interpretado ([`netmap`]), o DERP ([`derp`]), o descobrimento de
//! caminho direto ([`disco`], [`stun`] e [`magicsock`]), o túnel WireGuard (do crate
//! `tomni-ui-wireguard`) e a pilha TCP/IP em software (`tomni-ui-netstack`).
//!
//! **Nada é exposto ao computador.** O único acesso à tailnet é o [`Tailscale::proxy`], um
//! SOCKS5 em `127.0.0.1` com porta e credencial aleatórias que só os clientes do app usam
//! (SSH, SFTP, WebView). O nó vem com o equivalente a `--shields-up`: não atende conexão
//! nenhuma que chegue da tailnet, nem anuncia sub-rede ou saída. Os nomes do MagicDNS
//! resolvem dentro do proxy, sem tocar no DNS do sistema.
//!
//! # Um nó em poucas linhas
//!
//! ```no_run
//! # async fn exemplo() -> std::io::Result<()> {
//! use std::time::Duration;
//! use tomni_ui_tailscale::{NodeState, State, TailscaleBuilder};
//!
//! // O `NodeState` guarda as chaves do nó: grave-o (cifrado) pelo `on_node_state` e passe-o
//! // de volta na próxima vez para continuar sendo a mesma máquina da tailnet.
//! let ts = TailscaleBuilder::new("https://controle.exemplo.com", NodeState::default())
//!     .hostname("meu-app")
//!     .on_state(|estado| {
//!         if let State::NeedsLogin { url } = estado {
//!             println!("abra {url} no navegador para entrar");
//!         }
//!     })
//!     .start()
//!     .await?;
//! let meu_ip = ts.wait_connected(Duration::from_secs(120)).await?;
//! println!("conectado como {meu_ip}");
//! // Os clientes do app usam `ts.proxy()` para chegar às máquinas, por IP ou por nome.
//! let _proxy = ts.proxy();
//! ts.stop(Duration::from_secs(3)).await;
//! # Ok(())
//! # }
//! ```
//!
//! # Testes
//!
//! No CI, o `ControlFake` (um plano de controle em memória, no modo tailnet), o `DerpFake` e a
//! rede simulada do netstack cobrem o registro, o mapa, o DERP, o NAT e a troca de caminho.
//! O teste real, contra o headscale e o `tailscale` oficiais em Docker, fica em
//! `testes/tailscale/rodar.sh` (`tests/tailscale_real.rs`, `#[ignore]`).
//!
//! # Origem
//!
//! O desenho do protocolo (ts2021, tailcfg, DERP, disco, magicsock e o mapa) segue o do
//! Tailscale (BSD-3-Clause), e partes foram portadas para Rust. Veja o arquivo `NOTICE`.

pub mod control;
pub mod derp;
pub mod disco;
pub mod engine;
pub mod key;
pub mod magicsock;
pub mod nacl;
pub mod netmap;
pub mod stun;
pub mod tailcfg;
pub mod upgrade;

pub use engine::{NodeState, Peer, State, Tailscale, TailscaleBuilder};

#[cfg(any(test, feature = "test-util"))]
pub mod testing;
