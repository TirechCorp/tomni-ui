//! Ajudas dos testes de dentro do crate: as credenciais das fixtures.

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::{Config, Credentials};

/// 2027-01-01, dentro da validade das fixtures, em nanossegundos desde 1970.
pub(crate) const AGORA_NANOS: u64 = 1_798_761_600 * 1_000_000_000;

/// A pasta das fixtures.
pub(crate) fn pasta() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// As credenciais do nó `no` (arquivos `<no>.crt` e `<no>.key`), na CA `ca`, com a cifra `cipher`
/// (`aes` ou `chachapoly`) e, se há, a versão com que inicia.
pub(crate) fn creds(no: &str, ca: &str, cipher: &str, iniciando: Option<u8>) -> Arc<Credentials> {
    let mut yaml = format!("pki:\n  ca: {ca}.crt\n  cert: {no}.crt\n  key: {no}.key\n");
    if let Some(versao) = iniciando {
        yaml += &format!("  initiating_version: {versao}\n");
    }
    yaml += &format!("cipher: {cipher}\n");
    let (config, _) = Config::from_yaml(&yaml, Some(&pasta())).expect("o config de teste");
    Arc::new(config.credentials().expect("as credenciais de teste"))
}
