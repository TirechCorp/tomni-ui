//! Monta e assina um planet (ou uma moon) de teste, para o `testes/zerotier/rodar.sh`: os
//! nós oficiais e o nosso cliente falam só com as raízes do teste, sem depender da internet.
//!
//! ```text
//! planet_teste SAIDA ASSINANTE.secret RAIZ.public=IP:PORTA[,IP:PORTA] [RAIZ.public=…]
//! planet_teste --moon ID_HEX SAIDA ASSINANTE.secret RAIZ.public=IP:PORTA …
//! ```
//!
//! O assinante é a identidade (com a parte secreta) que assina o mundo e as atualizações dele.
//! As raízes entram só com a parte pública. Chaves só de teste; nunca use numa rede de verdade.

use std::fs;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use tomni_ui_zerotier::world::{ID_TERRA, Root};
use tomni_ui_zerotier::{Identity, World, WorldKind};

fn ler_identidade(caminho: &str) -> Result<Identity, String> {
    let texto = fs::read_to_string(caminho).map_err(|e| format!("{caminho}: {e}"))?;
    Identity::from_secret_str(texto.trim())
        .or_else(|_| Identity::from_public_str(texto.trim()))
        .map_err(|e| format!("{caminho}: {e}"))
}

fn rodar() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let (tipo, id) = if args.first().map(String::as_str) == Some("--moon") {
        if args.len() < 2 {
            return Err("faltou o id da moon".into());
        }
        let id = u64::from_str_radix(&args[1], 16).map_err(|e| format!("id da moon: {e}"))?;
        args.drain(..2);
        (WorldKind::Moon, id)
    } else {
        (WorldKind::Planet, ID_TERRA)
    };
    if args.len() < 3 {
        return Err(
            "uso: planet_teste [--moon ID] SAIDA ASSINANTE.secret RAIZ.public=IP:PORTA…".into(),
        );
    }
    let saida = args.remove(0);
    let assinante = ler_identidade(&args.remove(0))?;
    let mut raizes = Vec::new();
    for arg in &args {
        let (arquivo, enderecos) = arg
            .split_once('=')
            .ok_or_else(|| format!("{arg}: esperava RAIZ.public=IP:PORTA"))?;
        let identity = ler_identidade(arquivo)?;
        let publica = Identity::from_public_str(&identity.to_public_string())
            .map_err(|e| format!("{arquivo}: {e}"))?;
        let endpoints = enderecos
            .split(',')
            .map(|e| e.parse::<SocketAddr>().map_err(|err| format!("{e}: {err}")))
            .collect::<Result<Vec<_>, _>>()?;
        raizes.push(Root {
            identity: publica,
            endpoints,
        });
    }
    let agora = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0));
    let mundo =
        World::make(tipo, id, agora, &assinante, raizes, &assinante).map_err(|e| e.to_string())?;
    fs::write(&saida, mundo.to_bytes()).map_err(|e| format!("{saida}: {e}"))?;
    println!(
        "{} {:x} ({} bytes) em {saida}",
        if tipo == WorldKind::Planet {
            "planet"
        } else {
            "moon"
        },
        id,
        mundo.to_bytes().len()
    );
    Ok(())
}

fn main() -> ExitCode {
    match rodar() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("planet_teste: {e}");
            ExitCode::FAILURE
        }
    }
}
