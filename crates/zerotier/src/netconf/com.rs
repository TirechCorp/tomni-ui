//! O certificado de pertinência (COM, `CertificateOfMembership`): a prova, assinada pelo
//! controlador da rede, de que um nó é membro. Dois membros só falam se os certificados
//! concordam (o carimbo de tempo perto, a mesma rede, o certificado emitido ao dono e o hash da
//! identidade de cada um).

use sha2::{Digest, Sha384};

use crate::identity::SIGNATURE_LEN;
use crate::{Address, Identity, NetworkId};

/// O máximo de qualificadores (`ZT_NETWORK_COM_MAX_ENTRIES`).
const MAX_QUALIFIERS: usize = 8;

/// Qualificador: o carimbo de tempo, com a janela de tolerância no `max_delta`.
pub const QUALIFIER_TIMESTAMP: u64 = 0;
/// Qualificador: o id da rede.
pub const QUALIFIER_NETWORK_ID: u64 = 1;
/// Qualificador: o endereço a quem o certificado foi emitido.
pub const QUALIFIER_ISSUED_TO: u64 = 2;
/// O primeiro dos quatro qualificadores com o hash da identidade do dono.
pub const QUALIFIER_IDENTITY: u64 = 3;

/// Um qualificador: `(id, valor, tolerância)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Qualifier {
    /// O que o valor mede.
    pub id: u64,
    /// O valor.
    pub value: u64,
    /// O quanto o valor do outro pode diferir.
    pub max_delta: u64,
}

/// O certificado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Com {
    qualifiers: Vec<Qualifier>,
    signed_by: Address,
    signature: Option<[u8; SIGNATURE_LEN]>,
}

/// Os 4 inteiros que representam uma identidade nos qualificadores 3 a 6: o SHA-384 do endereço
/// e da chave pública, em pedaços de 8 bytes (big-endian).
fn identity_words(id: &Identity) -> [u64; 4] {
    let mut h = Sha384::new();
    h.update(id.address().to_bytes());
    h.update(id.public_bytes());
    let hash = h.finalize();
    let mut palavras = [0u64; 4];
    for (i, p) in palavras.iter_mut().enumerate() {
        *p = u64::from_be_bytes(hash[i * 8..i * 8 + 8].try_into().expect("8 bytes"));
    }
    palavras
}

impl Com {
    /// Emite (e assina) um certificado, como o controlador faz. Usado pelo `ControllerFake`.
    /// `None` se `controlador` não tem a parte secreta.
    pub fn issue(
        controlador: &Identity,
        rede: NetworkId,
        dono: &Identity,
        timestamp: u64,
        janela: u64,
    ) -> Option<Self> {
        let w = identity_words(dono);
        let mut qualifiers = vec![
            Qualifier {
                id: QUALIFIER_TIMESTAMP,
                value: timestamp,
                max_delta: janela,
            },
            Qualifier {
                id: QUALIFIER_NETWORK_ID,
                value: rede.as_u64(),
                max_delta: 0,
            },
            Qualifier {
                id: QUALIFIER_ISSUED_TO,
                value: dono.address().as_u64(),
                max_delta: u64::MAX,
            },
        ];
        for (i, valor) in w.iter().enumerate() {
            qualifiers.push(Qualifier {
                id: QUALIFIER_IDENTITY + i as u64,
                value: *valor,
                max_delta: u64::MAX,
            });
        }
        let mut com = Self {
            qualifiers,
            signed_by: controlador.address(),
            signature: None,
        };
        com.signature = Some(controlador.sign(&com.signed_bytes())?);
        Some(com)
    }

    /// Os bytes que o controlador assina: `(id, valor, delta)` de cada qualificador, em
    /// big-endian.
    fn signed_bytes(&self) -> Vec<u8> {
        let mut saida = Vec::with_capacity(self.qualifiers.len() * 24);
        for q in &self.qualifiers {
            saida.extend_from_slice(&q.id.to_be_bytes());
            saida.extend_from_slice(&q.value.to_be_bytes());
            saida.extend_from_slice(&q.max_delta.to_be_bytes());
        }
        saida
    }

    /// Escreve no formato do fio.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut saida = vec![1u8];
        saida.extend_from_slice(&(self.qualifiers.len() as u16).to_be_bytes());
        for q in &self.qualifiers {
            saida.extend_from_slice(&q.id.to_be_bytes());
            saida.extend_from_slice(&q.value.to_be_bytes());
            saida.extend_from_slice(&q.max_delta.to_be_bytes());
        }
        saida.extend_from_slice(&self.signed_by.to_bytes());
        if self.signed_by.as_u64() != 0
            && let Some(assinatura) = &self.signature
        {
            saida.extend_from_slice(assinatura);
        }
        saida
    }

    /// Lê do fio. Devolve o certificado e quantos bytes consumiu. `None` se está truncado, é de
    /// outra versão ou tem qualificadores demais ou fora de ordem.
    pub fn from_bytes(bytes: &[u8]) -> Option<(Self, usize)> {
        if *bytes.first()? != 1 {
            return None;
        }
        let n = usize::from(u16::from_be_bytes([*bytes.get(1)?, *bytes.get(2)?]));
        if n > MAX_QUALIFIERS {
            return None;
        }
        let mut p = 3;
        let mut qualifiers = Vec::with_capacity(n);
        let mut anterior = 0u64;
        for i in 0..n {
            let b = bytes.get(p..p + 24)?;
            let id = u64::from_be_bytes(b[0..8].try_into().ok()?);
            if i > 0 && id < anterior {
                return None;
            }
            anterior = id;
            qualifiers.push(Qualifier {
                id,
                value: u64::from_be_bytes(b[8..16].try_into().ok()?),
                max_delta: u64::from_be_bytes(b[16..24].try_into().ok()?),
            });
            p += 24;
        }
        let signed_by = Address::from_bytes(bytes.get(p..p + 5)?)?;
        p += 5;
        let signature = if signed_by.as_u64() != 0 {
            let s: [u8; SIGNATURE_LEN] = bytes.get(p..p + SIGNATURE_LEN)?.try_into().ok()?;
            p += SIGNATURE_LEN;
            Some(s)
        } else {
            None
        };
        Some((
            Self {
                qualifiers,
                signed_by,
                signature,
            },
            p,
        ))
    }

    /// Quem assinou.
    pub fn signed_by(&self) -> Address {
        self.signed_by
    }

    /// Os qualificadores.
    pub fn qualifiers(&self) -> &[Qualifier] {
        &self.qualifiers
    }

    /// O valor de um qualificador.
    pub fn value(&self, id: u64) -> Option<u64> {
        self.qualifiers.iter().find(|q| q.id == id).map(|q| q.value)
    }

    /// A rede, se o certificado traz o qualificador.
    pub fn network(&self) -> Option<NetworkId> {
        self.value(QUALIFIER_NETWORK_ID).map(NetworkId::new)
    }

    /// A quem foi emitido.
    pub fn issued_to(&self) -> Option<Address> {
        self.value(QUALIFIER_ISSUED_TO).map(Address::new)
    }

    /// O carimbo de tempo.
    pub fn timestamp(&self) -> Option<u64> {
        self.value(QUALIFIER_TIMESTAMP)
    }

    /// A assinatura confere com o controlador da rede? `controlador` é a identidade dele.
    pub fn verify(&self, rede: NetworkId, controlador: &Identity) -> bool {
        if self.qualifiers.len() > MAX_QUALIFIERS
            || self.signed_by != rede.controller()
            || controlador.address() != self.signed_by
        {
            return false;
        }
        match &self.signature {
            Some(s) => controlador.verify(&self.signed_bytes(), s),
            None => false,
        }
    }

    /// Este certificado (o nosso, ou o que vamos conferir) concorda com o `outro`, que pertence
    /// a `identidade_do_outro`? Todo qualificador nosso precisa existir no outro com diferença
    /// de até `max_delta`; os de identidade precisam bater com o hash da identidade do outro.
    pub fn agrees_with(&self, outro: &Com, identidade_do_outro: &Identity) -> bool {
        if self.qualifiers.is_empty() || outro.qualifiers.is_empty() {
            return false;
        }
        let palavras = identity_words(identidade_do_outro);
        for q in &self.qualifiers {
            let Some(o) = outro.qualifiers.iter().find(|o| o.id == q.id) else {
                return false;
            };
            if q.value.abs_diff(o.value) > q.max_delta {
                return false;
            }
            if (QUALIFIER_IDENTITY..QUALIFIER_IDENTITY + 4).contains(&q.id) {
                let i = (q.id - QUALIFIER_IDENTITY) as usize;
                if o.value != palavras[i] {
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn identidade(n: u8) -> Identity {
        let texto = std::fs::read_to_string(format!(
            "{}/tests/fixtures/identidades/id{n}.secret",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        Identity::from_secret_str(&texto).unwrap()
    }

    fn rede_de(controlador: &Identity) -> NetworkId {
        NetworkId::new((controlador.address().as_u64() << 24) | 1)
    }

    #[test]
    fn o_certificado_vai_e_volta_pelo_fio_e_a_assinatura_confere() {
        let ctl = identidade(1);
        let nwid = rede_de(&ctl);
        let com = Com::issue(&ctl, nwid, &identidade(2), 1_000_000, 600_000).unwrap();
        let bytes = com.to_bytes();
        let (lido, usados) = Com::from_bytes(&bytes).unwrap();
        assert_eq!(usados, bytes.len());
        assert_eq!(lido, com);
        assert!(lido.verify(nwid, &ctl));
        assert_eq!(lido.network(), Some(nwid));
        assert_eq!(lido.issued_to(), Some(identidade(2).address()));
    }

    #[test]
    fn certificado_adulterado_ou_de_outra_rede_nao_confere() {
        let ctl = identidade(1);
        let nwid = rede_de(&ctl);
        let mut bytes = Com::issue(&ctl, nwid, &identidade(2), 5, 10)
            .unwrap()
            .to_bytes();
        bytes[20] ^= 1;
        let (adulterado, _) = Com::from_bytes(&bytes).unwrap();
        assert!(!adulterado.verify(nwid, &ctl));

        let com = Com::issue(&ctl, nwid, &identidade(2), 5, 10).unwrap();
        assert!(!com.verify(NetworkId::new(nwid.as_u64() + (1 << 24)), &ctl));
        assert!(!com.verify(nwid, &identidade(2)));
    }

    #[test]
    fn dois_membros_concordam_se_o_tempo_esta_perto_e_a_identidade_bate() {
        let ctl = identidade(1);
        let nwid = rede_de(&ctl);
        let (a, b) = (identidade(2), identidade(3));
        let com_a = Com::issue(&ctl, nwid, &a, 1_000, 500).unwrap();
        let com_b = Com::issue(&ctl, nwid, &b, 1_300, 500).unwrap();
        assert!(com_a.agrees_with(&com_b, &b));
        assert!(com_b.agrees_with(&com_a, &a));
        // Identidade trocada: o hash não bate.
        assert!(!com_a.agrees_with(&com_b, &a));
        // Tempo longe demais.
        let velho = Com::issue(&ctl, nwid, &b, 100_000, 500).unwrap();
        assert!(!com_a.agrees_with(&velho, &b));
    }

    #[test]
    fn de_outra_rede_nao_concorda() {
        let ctl = identidade(1);
        let (a, b) = (identidade(2), identidade(3));
        let nwid = rede_de(&ctl);
        let outra = NetworkId::new(nwid.as_u64() + 1);
        let com_a = Com::issue(&ctl, nwid, &a, 1_000, 500).unwrap();
        let com_b = Com::issue(&ctl, outra, &b, 1_000, 500).unwrap();
        assert!(!com_a.agrees_with(&com_b, &b));
    }

    #[test]
    fn lixo_nao_vira_certificado() {
        assert!(Com::from_bytes(&[]).is_none());
        assert!(Com::from_bytes(&[2, 0, 0]).is_none(), "versão desconhecida");
        assert!(
            Com::from_bytes(&[1, 0, 9]).is_none(),
            "qualificadores demais"
        );
        assert!(Com::from_bytes(&[1, 0, 1, 0]).is_none(), "truncado");
    }
}
