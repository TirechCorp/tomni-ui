//! Utilidades dos testes: as fixtures de `tests/fixtures/` (geradas por
//! `testes/openvpn/gerar-fixtures.sh`; as chaves são só de teste).
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

/// A pasta das fixtures.
pub fn pasta() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Lê uma fixture de texto.
pub fn ler(nome: &str) -> String {
    std::fs::read_to_string(pasta().join(nome))
        .unwrap_or_else(|e| panic!("não leu a fixture {nome}: {e}"))
}

/// Uma hora dentro da validade de todas as fixtures, exceto a do certificado vencido
/// (as folhas valem de 2026-10-22 a 2036-10-20).
pub fn agora() -> std::time::SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

/// O `referencias.txt` (`arquivo.campo=valor`), calculado pelo `openssl` na geração.
pub fn referencias() -> HashMap<String, String> {
    ler("referencias.txt")
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

pub mod mundo;
pub mod rede;
