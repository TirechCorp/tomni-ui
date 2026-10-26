//! As chaves do Tailscale: a da máquina, a do nó e a de disco.
//!
//! No texto (JSON e `.conf` do controle) cada uma tem um prefixo e 64 dígitos hexadecimais
//! minúsculos: `mkey:`, `nodekey:`, `discokey:`, e `chalpub:` e `tlpub:` para as duas que só
//! passam pelo controle. A privada de cada uma se escreve com o prefixo `privkey:`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use x25519_dalek::{PublicKey as Publica, StaticSecret};

/// O tamanho de qualquer chave, em bytes.
pub const KEY_LEN: usize = 32;

/// O prefixo de uma chave privada no texto.
const PREFIXO_PRIVADA: &str = "privkey:";

/// O que pode dar errado ao ler uma chave do texto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// O texto não começa com o prefixo esperado.
    Prefixo(&'static str),
    /// Depois do prefixo não vêm 64 dígitos hexadecimais.
    Hex,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::Prefixo(p) => write!(f, "a chave precisa começar com \"{p}\""),
            KeyError::Hex => f.write_str("a chave precisa ter 64 dígitos hexadecimais"),
        }
    }
}

impl std::error::Error for KeyError {}

/// Escreve os bytes em hexadecimal minúsculo.
fn para_hex(bytes: &[u8]) -> String {
    const DIGITOS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITOS[(b >> 4) as usize] as char);
        s.push(DIGITOS[(b & 15) as usize] as char);
    }
    s
}

/// Lê 64 dígitos hexadecimais (maiúsculos ou minúsculos).
fn de_hex(texto: &str) -> Result<[u8; KEY_LEN], KeyError> {
    let b = texto.as_bytes();
    if b.len() != KEY_LEN * 2 {
        return Err(KeyError::Hex);
    }
    let dig = |c: u8| match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(KeyError::Hex),
    };
    let mut saida = [0u8; KEY_LEN];
    for (i, par) in b.chunks_exact(2).enumerate() {
        saida[i] = (dig(par[0])? << 4) | dig(par[1])?;
    }
    Ok(saida)
}

/// Gera 32 bytes aleatórios.
fn aleatorio() -> [u8; KEY_LEN] {
    let mut b = [0u8; KEY_LEN];
    getrandom::fill(&mut b).expect("o sistema não deu números aleatórios");
    b
}

/// Declara uma chave pública com o seu prefixo de texto.
macro_rules! chave_publica {
    ($(#[$doc:meta])* $nome:ident, $prefixo:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $nome([u8; KEY_LEN]);

        impl $nome {
            /// O prefixo no texto.
            pub const PREFIX: &'static str = $prefixo;

            /// Monta a chave de 32 bytes.
            pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
                Self(bytes)
            }

            /// Os 32 bytes da chave.
            pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
                &self.0
            }

            /// Se é a chave zero (a que o controle usa para "sem chave").
            pub fn is_zero(&self) -> bool {
                self.0 == [0u8; KEY_LEN]
            }
        }

        impl fmt::Display for $nome {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefixo, para_hex(&self.0))
            }
        }

        impl fmt::Debug for $nome {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefixo, &para_hex(&self.0)[..5])
            }
        }

        impl FromStr for $nome {
            type Err = KeyError;

            fn from_str(s: &str) -> Result<Self, KeyError> {
                let resto = s.strip_prefix($prefixo).ok_or(KeyError::Prefixo($prefixo))?;
                Ok(Self(de_hex(resto)?))
            }
        }

        impl Serialize for $nome {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $nome {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let texto = <std::borrow::Cow<'de, str>>::deserialize(d)?;
                texto.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

/// Declara uma chave privada X25519 (apagada da memória ao sair de cena).
macro_rules! chave_privada {
    ($(#[$doc:meta])* $nome:ident, $publica:ident) => {
        $(#[$doc])*
        pub struct $nome(StaticSecret);

        impl $nome {
            /// Gera uma chave nova.
            pub fn generate() -> Self {
                Self::from_bytes(aleatorio())
            }

            /// Monta a chave de 32 bytes (o X25519 faz o ajuste dos bits sozinho).
            pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
                Self(StaticSecret::from(bytes))
            }

            /// Os 32 bytes da chave. Cuidado com onde isto vai parar.
            pub fn to_bytes(&self) -> [u8; KEY_LEN] {
                self.0.to_bytes()
            }

            /// A chave pública.
            pub fn public(&self) -> $publica {
                $publica::from_bytes(Publica::from(&self.0).to_bytes())
            }

            /// O texto `privkey:…` para guardar (cifrado pelo app, nunca em claro por aí).
            pub fn to_text(&self) -> String {
                format!("{PREFIXO_PRIVADA}{}", para_hex(&self.to_bytes()))
            }

            /// Lê o texto `privkey:…`.
            pub fn from_text(texto: &str) -> Result<Self, KeyError> {
                let resto = texto
                    .strip_prefix(PREFIXO_PRIVADA)
                    .ok_or(KeyError::Prefixo(PREFIXO_PRIVADA))?;
                Ok(Self::from_bytes(de_hex(resto)?))
            }
        }

        impl Clone for $nome {
            fn clone(&self) -> Self {
                Self::from_bytes(self.to_bytes())
            }
        }

        impl fmt::Debug for $nome {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // Nunca mostra o segredo: só a pública.
                write!(f, concat!(stringify!($nome), "({:?})"), self.public())
            }
        }
    };
}

chave_publica!(
    /// A chave pública da máquina: identifica o aparelho para o controle (`mkey:`).
    MachinePublic,
    "mkey:"
);
chave_publica!(
    /// A chave pública do nó: a do WireGuard (`nodekey:`).
    NodePublic,
    "nodekey:"
);
chave_publica!(
    /// A chave pública de disco, para o NAT traversal (`discokey:`).
    DiscoPublic,
    "discokey:"
);
chave_publica!(
    /// A chave pública de desafio que o controle manda no início da conexão (`chalpub:`).
    ChallengePublic,
    "chalpub:"
);
chave_publica!(
    /// A chave pública do bloqueio de rede (tailnet lock, `tlpub:`). Aqui só vai zerada.
    LockPublic,
    "tlpub:"
);

chave_privada!(
    /// A chave privada da máquina.
    MachineKey,
    MachinePublic
);
chave_privada!(
    /// A chave privada do nó (a do WireGuard).
    NodeKey,
    NodePublic
);
chave_privada!(
    /// A chave privada de disco.
    DiscoKey,
    DiscoPublic
);

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn o_texto_vai_e_volta() {
        let k = NodeKey::generate();
        let p = k.public();
        let texto = p.to_string();
        assert!(texto.starts_with("nodekey:"));
        assert_eq!(texto.len(), "nodekey:".len() + 64);
        assert_eq!(texto.parse::<NodePublic>().unwrap(), p);
        let privada = NodeKey::from_text(&k.to_text()).unwrap();
        assert_eq!(privada.public(), p);
    }

    #[test]
    fn prefixo_errado_e_hex_ruim_dao_erro() {
        let m = MachineKey::generate().public().to_string();
        assert_eq!(m.parse::<NodePublic>(), Err(KeyError::Prefixo("nodekey:")));
        assert_eq!("mkey:xyz".parse::<MachinePublic>(), Err(KeyError::Hex));
        let longa = format!("mkey:{}", "a".repeat(66));
        assert_eq!(longa.parse::<MachinePublic>(), Err(KeyError::Hex));
        assert!(
            format!("mkey:{}", "G".repeat(64))
                .parse::<MachinePublic>()
                .is_err()
        );
    }

    #[test]
    fn a_zero_se_reconhece_e_vai_para_o_json() {
        let zero = NodePublic::default();
        assert!(zero.is_zero());
        let json = serde_json::to_string(&zero).unwrap();
        assert_eq!(json, format!("\"nodekey:{}\"", "0".repeat(64)));
        let volta: NodePublic = serde_json::from_str(&json).unwrap();
        assert!(volta.is_zero());
    }

    #[test]
    fn o_debug_nao_mostra_o_segredo() {
        let k = MachineKey::generate();
        let texto = format!("{k:?}");
        let segredo = k.to_text();
        let hex = segredo.trim_start_matches("privkey:");
        assert!(!texto.contains(hex));
        assert!(texto.starts_with("MachineKey(mkey:"));
    }

    #[test]
    fn a_pública_conhecida_do_x25519_bate() {
        // Vetor do RFC 7748, seção 6.1 (Alice).
        let privada = [
            0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2,
            0x66, 0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5,
            0x1d, 0xb9, 0x2c, 0x2a,
        ];
        let esperada = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
        let p = DiscoKey::from_bytes(privada).public();
        assert_eq!(p.to_string(), format!("discokey:{esperada}"));
    }
}
