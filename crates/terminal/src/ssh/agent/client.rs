//! O cliente do agente: fala o protocolo com qualquer `ssh-agent` (o do sistema, o do
//! [`Agent`](super::Agent) do app, o do OpenSSH no Windows). É bloqueante e curto, para as telas e
//! os programas de linha de comando.

use std::io::{self, Read, Write};
use std::path::Path;
#[cfg(unix)]
use std::time::Duration;

use ssh_key::encoding::Encode;
use ssh_key::{PrivateKey, PublicKey};

use super::Constraints;
use super::wire::{self, Reader, Writer};

/// Quanto o cliente espera uma resposta (uma confirmação de uso cabe aqui).
#[cfg(unix)]
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);

trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

/// Uma conexão com um agente de chaves.
pub struct AgentConnection {
    stream: Box<dyn Stream>,
}

impl std::fmt::Debug for AgentConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConnection").finish_non_exhaustive()
    }
}

fn other(mensagem: impl Into<String>) -> io::Error {
    io::Error::other(mensagem.into())
}

impl AgentConnection {
    /// Conecta ao socket (Unix) ou ao pipe nomeado (Windows) em `path`.
    pub fn connect(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let stream = std::os::unix::net::UnixStream::connect(path)?;
            stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
            stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
            Ok(Self {
                stream: Box::new(stream),
            })
        }
        #[cfg(not(unix))]
        {
            // Um pipe nomeado abre como um arquivo comum.
            let stream = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?;
            Ok(Self {
                stream: Box::new(stream),
            })
        }
    }

    /// Conecta ao agente do ambiente: o `SSH_AUTH_SOCK` (no Windows, sem ele, o pipe do agente do
    /// OpenSSH).
    pub fn from_env() -> io::Result<Self> {
        match std::env::var_os("SSH_AUTH_SOCK").filter(|v| !v.is_empty()) {
            Some(caminho) => Self::connect(Path::new(&caminho)),
            #[cfg(windows)]
            None => Self::connect(Path::new(super::OPENSSH_PIPE)),
            #[cfg(not(windows))]
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "A variável SSH_AUTH_SOCK não está definida.",
            )),
        }
    }

    fn call(&mut self, pedido: &[u8]) -> io::Result<Vec<u8>> {
        self.stream.write_all(&wire::framed(pedido))?;
        self.stream.flush()?;
        let mut tamanho = [0u8; 4];
        self.stream.read_exact(&mut tamanho)?;
        let tamanho = u32::from_be_bytes(tamanho) as usize;
        if tamanho == 0 || tamanho > wire::MAX_FRAME {
            return Err(other("O agente mandou uma resposta inválida."));
        }
        let mut resposta = vec![0u8; tamanho];
        self.stream.read_exact(&mut resposta)?;
        Ok(resposta)
    }

    /// Manda o pedido e espera `SUCCESS`.
    fn call_ok(&mut self, pedido: &[u8], recusa: &str) -> io::Result<()> {
        match self.call(pedido)?.first() {
            Some(&wire::SUCCESS) => Ok(()),
            Some(&wire::FAILURE) => Err(other(recusa)),
            _ => Err(other("O agente deu uma resposta que não se esperava.")),
        }
    }

    /// As chaves do agente: a pública e o comentário. Chaves que este cliente não entende (os
    /// certificados) ficam de fora.
    pub fn list(&mut self) -> io::Result<Vec<(PublicKey, String)>> {
        let resposta = self.call(&[wire::REQUEST_IDENTITIES])?;
        let mut r = Reader(&resposta);
        if r.u8() != Some(wire::IDENTITIES_ANSWER) {
            return Err(other("O agente recusou listar as chaves."));
        }
        let quantas = r
            .u32()
            .ok_or_else(|| other("Resposta do agente cortada."))?;
        let mut chaves = Vec::new();
        for _ in 0..quantas {
            let (Some(blob), Some(comentario)) = (r.string(), r.string()) else {
                return Err(other("Resposta do agente cortada."));
            };
            if let Ok(publica) = PublicKey::from_bytes(blob) {
                chaves.push((publica, String::from_utf8_lossy(comentario).into_owned()));
            }
        }
        Ok(chaves)
    }

    /// Põe a chave no agente (a privada precisa estar aberta, sem passphrase).
    pub fn add(&mut self, key: &PrivateKey, comment: &str, c: Constraints) -> io::Result<()> {
        let com_restricoes = c.lifetime.is_some() || c.confirm;
        let mut w = Writer::new(if com_restricoes {
            wire::ADD_ID_CONSTRAINED
        } else {
            wire::ADD_IDENTITY
        });
        key.key_data()
            .encode(&mut w.0)
            .map_err(|e| other(format!("A chave não pôde ser enviada: {e}")))?;
        w.string(comment.as_bytes());
        if let Some(vida) = c.lifetime {
            // O protocolo conta em segundos inteiros: o que sobra de um segundo vale um segundo.
            let segundos = vida.as_secs() + u64::from(vida.subsec_nanos() > 0);
            w.u8(wire::CONSTRAIN_LIFETIME)
                .u32(segundos.min(u32::MAX.into()) as u32);
        }
        if c.confirm {
            w.u8(wire::CONSTRAIN_CONFIRM);
        }
        self.call_ok(&w.0, "O agente recusou a chave.")
    }

    /// Tira a chave do agente.
    pub fn remove(&mut self, public: &PublicKey) -> io::Result<()> {
        let blob = public
            .to_bytes()
            .map_err(|e| other(format!("A chave não pôde ser enviada: {e}")))?;
        let mut w = Writer::new(wire::REMOVE_IDENTITY);
        w.string(&blob);
        self.call_ok(&w.0, "O agente não tem essa chave.")
    }

    /// Tira todas as chaves do agente.
    pub fn remove_all(&mut self) -> io::Result<()> {
        self.call_ok(&[wire::REMOVE_ALL_IDENTITIES], "O agente recusou o pedido.")
    }

    /// Trava o agente com a passphrase.
    pub fn lock(&mut self, passphrase: &str) -> io::Result<()> {
        let mut w = Writer::new(wire::LOCK);
        w.string(passphrase.as_bytes());
        self.call_ok(&w.0, "O agente recusou travar (já está travado?).")
    }

    /// Destrava o agente.
    pub fn unlock(&mut self, passphrase: &str) -> io::Result<()> {
        let mut w = Writer::new(wire::UNLOCK);
        w.string(passphrase.as_bytes());
        self.call_ok(&w.0, "A passphrase não confere.")
    }

    /// Pede a assinatura de `data` com a chave `public` (para os testes e as ferramentas).
    /// `flags`: 2 = `rsa-sha2-256`, 4 = `rsa-sha2-512`.
    pub fn sign(&mut self, public: &PublicKey, data: &[u8], flags: u32) -> io::Result<Vec<u8>> {
        let blob = public
            .to_bytes()
            .map_err(|e| other(format!("A chave não pôde ser enviada: {e}")))?;
        let mut w = Writer::new(wire::SIGN_REQUEST);
        w.string(&blob).string(data).u32(flags);
        let resposta = self.call(&w.0)?;
        let mut r = Reader(&resposta);
        match r.u8() {
            Some(wire::SIGN_RESPONSE) => r
                .string()
                .map(<[u8]>::to_vec)
                .ok_or_else(|| other("Resposta do agente cortada.")),
            Some(wire::FAILURE) => Err(other("O agente recusou assinar.")),
            _ => Err(other("O agente deu uma resposta que não se esperava.")),
        }
    }
}
