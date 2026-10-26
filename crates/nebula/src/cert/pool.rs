//! O conjunto de CAs em que a rede confia e a conferência dos certificados dos nós.

use std::collections::{HashMap, HashSet};

use super::{CertError, Certificate, Curve};

/// As CAs do `pki.ca` e a lista de bloqueio do `pki.blocklist`.
#[derive(Clone, Debug, Default)]
pub struct CaPool {
    /// A impressão digital (hexadecimal) → a CA.
    cas: HashMap<String, Certificate>,
    blocklist: HashSet<String>,
}

impl CaPool {
    /// Um conjunto vazio.
    pub fn new() -> CaPool {
        CaPool::default()
    }

    /// Lê as CAs de um texto PEM (uma ou mais).
    pub fn from_pem(texto: &str) -> Result<CaPool, CertError> {
        let mut pool = CaPool::new();
        pool.add_pem(texto)?;
        Ok(pool)
    }

    /// Acrescenta as CAs de um texto PEM e devolve quantas eram.
    pub fn add_pem(&mut self, texto: &str) -> Result<usize, CertError> {
        let certificados = Certificate::from_pem(texto)?;
        let total = certificados.len();
        for cert in certificados {
            self.add(cert)?;
        }
        Ok(total)
    }

    /// Acrescenta uma CA. Ela precisa ser de CA e estar assinada por ela mesma. Uma CA vencida
    /// entra (o `verify` recusa os certificados dela com a mensagem do vencimento).
    pub fn add(&mut self, ca: Certificate) -> Result<(), CertError> {
        if !ca.is_ca() {
            return Err(CertError::NotCa);
        }
        // A autoassinatura de uma CA P-256 não é conferida (a P-256 ainda não é suportada, e o
        // `verify` dos nós dela recusa com a mensagem certa).
        if ca.curve() == Curve::Curve25519 && ca.check_signature(ca.public_key()).is_err() {
            return Err(CertError::NotSelfSigned);
        }
        self.cas.insert(ca.fingerprint(), ca);
        Ok(())
    }

    /// Bloqueia um certificado pela impressão digital (hexadecimal).
    pub fn block(&mut self, fingerprint: &str) {
        self.blocklist
            .insert(fingerprint.trim().to_ascii_lowercase());
    }

    /// Quantas CAs há.
    pub fn len(&self) -> usize {
        self.cas.len()
    }

    /// `true` se não há nenhuma CA.
    pub fn is_empty(&self) -> bool {
        self.cas.is_empty()
    }

    /// Confere o certificado de um nó no instante `agora` (segundos desde 1970), na ordem do
    /// Nebula: bloqueio, CA, curva, validade da CA e do certificado, assinatura e restrições.
    pub fn verify(&self, cert: &Certificate, agora: i64) -> Result<(), CertError> {
        let nome = || cert.name().to_owned();
        if self.blocklist.contains(&cert.fingerprint()) {
            return Err(CertError::Blocklisted { name: nome() });
        }
        let ca = self
            .cas
            .get(&cert.issuer())
            .ok_or_else(|| CertError::CaNotFound { name: nome() })?;
        if ca.curve() != cert.curve() {
            return Err(CertError::CurveMismatch { name: nome() });
        }
        if ca.not_before() > agora {
            return Err(CertError::NotYetValid {
                name: ca.name().to_owned(),
                not_before: ca.not_before(),
            });
        }
        if ca.not_after() < agora {
            return Err(CertError::RootExpired {
                name: ca.name().to_owned(),
                not_after: ca.not_after(),
            });
        }
        if cert.not_before() > agora {
            return Err(CertError::NotYetValid {
                name: nome(),
                not_before: cert.not_before(),
            });
        }
        if cert.not_after() < agora {
            return Err(CertError::Expired {
                name: nome(),
                not_after: cert.not_after(),
            });
        }
        cert.check_signature(ca.public_key())?;
        check_constraints(ca, cert)
    }
}

/// O que a CA limita no que assina (`CheckCAConstraints` do Nebula).
pub(super) fn check_constraints(ca: &Certificate, cert: &Certificate) -> Result<(), CertError> {
    let erro = |motivo: String| CertError::Constraints {
        name: cert.name().to_owned(),
        reason: motivo,
    };
    if cert.not_after() > ca.not_after() {
        return Err(erro("vence depois da CA".to_owned()));
    }
    if cert.not_before() < ca.not_before() {
        return Err(erro("vale desde antes da CA".to_owned()));
    }
    if !ca.groups().is_empty() {
        for grupo in cert.groups() {
            if !ca.groups().contains(grupo) {
                return Err(erro(format!("o grupo {grupo} não está na CA")));
            }
        }
    }
    let dentro = |limites: &[tomni_ui_netstack::Cidr], rede: &tomni_ui_netstack::Cidr| {
        limites
            .iter()
            .any(|limite| limite.contains(rede.addr()) && limite.prefix() <= rede.prefix())
    };
    if !ca.networks().is_empty() {
        for rede in cert.networks() {
            if !dentro(ca.networks(), rede) {
                return Err(erro(format!("a rede {rede} está fora das redes da CA")));
            }
        }
    }
    if !ca.unsafe_networks().is_empty() {
        for rede in cert.unsafe_networks() {
            if !dentro(ca.unsafe_networks(), rede) {
                return Err(erro(format!(
                    "a rede de fora {rede} está fora das redes de fora da CA"
                )));
            }
        }
    }
    Ok(())
}
