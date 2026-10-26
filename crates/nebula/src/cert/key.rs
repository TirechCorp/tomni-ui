//! A chave privada do nó (`NEBULA X25519 PRIVATE KEY`).

use std::fmt;

use zeroize::Zeroizing;

use super::{CertError, Certificate, Curve, unsupported_p256};
use crate::pem;

/// A chave privada do nó. Os bytes ficam num buffer que se apaga ao sair da memória, e o
/// `Debug` não os mostra.
pub struct PrivateKey {
    curve: Curve,
    bytes: Zeroizing<Vec<u8>>,
}

impl PrivateKey {
    /// Lê uma chave de um texto PEM (`NEBULA X25519 PRIVATE KEY` ou `NEBULA P256 PRIVATE KEY`).
    pub fn from_pem(texto: &str) -> Result<PrivateKey, CertError> {
        let blocos = pem::decode(texto).map_err(CertError::Pem)?;
        for bloco in blocos {
            let curve = match bloco.label.as_str() {
                "NEBULA X25519 PRIVATE KEY" => Curve::Curve25519,
                "NEBULA P256 PRIVATE KEY" => Curve::P256,
                "NEBULA ED25519 PRIVATE KEY" => {
                    return Err(CertError::Pem(
                        "esta é a chave de uma CA, e não a de um nó".to_owned(),
                    ));
                }
                "NEBULA ENCRYPTED ED25519 PRIVATE KEY" => {
                    return Err(CertError::Unsupported(
                        "chave privada cifrada com senha ainda não é suportada".to_owned(),
                    ));
                }
                _ => continue,
            };
            let bytes = Zeroizing::new(bloco.data);
            if bytes.len() != 32 {
                return Err(CertError::Pem(format!(
                    "a chave privada deveria ter 32 bytes e tem {}",
                    bytes.len()
                )));
            }
            return Ok(PrivateKey { curve, bytes });
        }
        Err(CertError::Pem(
            "o texto não tem nenhum bloco NEBULA X25519 PRIVATE KEY".to_owned(),
        ))
    }

    /// Monta a chave de 32 bytes de uma curva (para os testes e o handshake).
    pub fn from_bytes(curve: Curve, bytes: [u8; 32]) -> PrivateKey {
        PrivateKey {
            curve,
            bytes: Zeroizing::new(bytes.to_vec()),
        }
    }

    /// A curva da chave.
    pub fn curve(&self) -> Curve {
        self.curve
    }

    /// Os bytes da chave, para o Noise.
    pub(crate) fn secret(&self) -> &[u8] {
        &self.bytes
    }

    /// A chave pública que corresponde a esta privada.
    pub fn public_key(&self) -> Result<[u8; 32], CertError> {
        match self.curve {
            Curve::Curve25519 => {
                let segredo: [u8; 32] =
                    self.bytes.as_slice().try_into().map_err(|_| {
                        CertError::Pem("a chave privada não tem 32 bytes".to_owned())
                    })?;
                Ok(x25519_dalek::x25519(
                    segredo,
                    x25519_dalek::X25519_BASEPOINT_BYTES,
                ))
            }
            Curve::P256 => Err(unsupported_p256()),
        }
    }

    /// Confere se a chave é a do certificado (mesma curva e mesma chave pública).
    pub fn matches(&self, cert: &Certificate) -> Result<(), CertError> {
        if cert.curve() != self.curve {
            return Err(CertError::KeyMismatch);
        }
        if self.public_key()?.as_slice() != cert.public_key() {
            return Err(CertError::KeyMismatch);
        }
        Ok(())
    }
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PrivateKey({}, ..)", self.curve)
    }
}

impl Clone for PrivateKey {
    fn clone(&self) -> PrivateKey {
        PrivateKey {
            curve: self.curve,
            bytes: Zeroizing::new(self.bytes.to_vec()),
        }
    }
}
