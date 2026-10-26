//! A chave estática do OpenVPN (`tls-auth`, `tls-crypt`).
//!
//! O arquivo é um texto com 256 bytes em hexadecimal, 16 por linha, entre as marcas
//! `-----BEGIN OpenVPN Static key V1-----` e `-----END OpenVPN Static key V1-----`; o que estiver
//! fora das marcas (os comentários do `openvpn --genkey`) é ignorado. Os 256 bytes são duas
//! chaves de 128 bytes, e cada uma tem 64 bytes de chave de cifra seguidos de 64 de chave de
//! HMAC (as origens: a descrição pública do protocolo, "OpenVPN Wire Protocol").

use std::fmt;

use zeroize::Zeroizing;

/// A marca de abertura do arquivo.
const BEGIN: &str = "-----BEGIN OpenVPN Static key V1-----";
/// A marca de fechamento do arquivo.
const END: &str = "-----END OpenVPN Static key V1-----";

/// O tamanho da chave estática: 2048 bits (formato do arquivo).
pub const STATIC_KEY_LEN: usize = 256;
/// O tamanho de cada metade (cifra mais HMAC) e de cada parte dentro dela (formato do arquivo).
const HALF_LEN: usize = STATIC_KEY_LEN / 2;
const PART_LEN: usize = HALF_LEN / 2;

/// O erro de ler uma chave estática.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticKeyError {
    /// Faltam as marcas `BEGIN`/`END` da chave estática.
    MissingMarkers,
    /// O corpo não é hexadecimal (posição do caractere ruim na linha dada, de 1 em diante).
    InvalidHex {
        /// A linha do arquivo.
        line: usize,
    },
    /// A chave não tem 256 bytes (o tamanho que veio).
    WrongSize(usize),
}

impl fmt::Display for StaticKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMarkers => {
                write!(
                    f,
                    "a chave estática não tem as marcas -----BEGIN OpenVPN Static key V1----- e -----END OpenVPN Static key V1-----"
                )
            }
            Self::InvalidHex { line } => {
                write!(
                    f,
                    "a chave estática tem caracteres que não são hexadecimais (linha {line})"
                )
            }
            Self::WrongSize(n) => {
                write!(
                    f,
                    "a chave estática deve ter 256 bytes (2048 bits), mas tem {n}"
                )
            }
        }
    }
}

impl std::error::Error for StaticKeyError {}

/// As duas partes de uma metade da chave: a de cifra e a de HMAC.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyHalf {
    /// 64 bytes de chave de cifra.
    pub cipher: Zeroizing<[u8; PART_LEN]>,
    /// 64 bytes de chave de HMAC.
    pub hmac: Zeroizing<[u8; PART_LEN]>,
}

impl fmt::Debug for KeyHalf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyHalf(..)")
    }
}

/// A chave estática de 2048 bits. O `Debug` não mostra o conteúdo e a memória é apagada ao
/// largar.
#[derive(Clone, PartialEq, Eq)]
pub struct StaticKey(Zeroizing<[u8; STATIC_KEY_LEN]>);

impl fmt::Debug for StaticKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StaticKey(..)")
    }
}

/// Qual metade da chave cada lado usa para enviar e receber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `key-direction` ausente: as duas pontas usam a metade 0 nos dois sentidos.
    Bidirectional,
    /// `key-direction 0`: envia com a metade 0 e recebe com a 1.
    Normal,
    /// `key-direction 1`: envia com a metade 1 e recebe com a 0.
    Inverse,
}

impl Direction {
    /// Converte o argumento do `key-direction` (0 ou 1; ausente é bidirecional).
    pub fn from_key_direction(value: Option<u8>) -> Direction {
        match value {
            None => Direction::Bidirectional,
            Some(0) => Direction::Normal,
            Some(_) => Direction::Inverse,
        }
    }
}

impl StaticKey {
    /// Monta a chave a partir dos 256 bytes.
    pub fn from_bytes(bytes: [u8; STATIC_KEY_LEN]) -> StaticKey {
        StaticKey(Zeroizing::new(bytes))
    }

    /// Lê o texto do arquivo (ou do bloco `<tls-auth>`/`<tls-crypt>`).
    pub fn from_pem(text: &str) -> Result<StaticKey, StaticKeyError> {
        let mut inside = false;
        let mut seen_begin = false;
        let mut seen_end = false;
        let mut bytes = Zeroizing::new(Vec::with_capacity(STATIC_KEY_LEN));
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if !inside {
                if line == BEGIN {
                    inside = true;
                    seen_begin = true;
                }
                continue;
            }
            if line == END {
                seen_end = true;
                break;
            }
            if line.is_empty() {
                continue;
            }
            if line.len() % 2 != 0 {
                return Err(StaticKeyError::InvalidHex { line: index + 1 });
            }
            for pair in line.as_bytes().chunks(2) {
                let high = hex_value(pair[0]);
                let low = hex_value(pair[1]);
                match (high, low) {
                    (Some(h), Some(l)) => bytes.push((h << 4) | l),
                    _ => return Err(StaticKeyError::InvalidHex { line: index + 1 }),
                }
            }
        }
        if !seen_begin || !seen_end {
            return Err(StaticKeyError::MissingMarkers);
        }
        let array: [u8; STATIC_KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| StaticKeyError::WrongSize(bytes.len()))?;
        Ok(StaticKey::from_bytes(array))
    }

    /// O texto do arquivo, no formato que o `openvpn --genkey` grava (sem os comentários).
    pub fn to_pem(&self) -> Zeroizing<String> {
        let mut out = Zeroizing::new(String::new());
        out.push_str(BEGIN);
        out.push('\n');
        for chunk in self.0.chunks(16) {
            for byte in chunk {
                out.push(char::from(HEX[usize::from(byte >> 4)]));
                out.push(char::from(HEX[usize::from(byte & 0x0f)]));
            }
            out.push('\n');
        }
        out.push_str(END);
        out.push('\n');
        out
    }

    /// Os 256 bytes.
    pub fn as_bytes(&self) -> &[u8; STATIC_KEY_LEN] {
        &self.0
    }

    /// A metade `index` (0 ou 1) da chave.
    ///
    /// # Panics
    /// Se `index` não for 0 nem 1.
    pub fn half(&self, index: usize) -> KeyHalf {
        assert!(index < 2, "a chave estática tem só duas metades");
        let base = index * HALF_LEN;
        let mut cipher = Zeroizing::new([0u8; PART_LEN]);
        let mut hmac = Zeroizing::new([0u8; PART_LEN]);
        cipher.copy_from_slice(&self.0[base..base + PART_LEN]);
        hmac.copy_from_slice(&self.0[base + PART_LEN..base + HALF_LEN]);
        KeyHalf { cipher, hmac }
    }

    /// As metades que o **cliente** usa para enviar e para receber no `tls-auth`, dado o
    /// `key-direction` do `.ovpn` (0 ou 1; ausente é bidirecional).
    pub fn tls_auth_halves(&self, direction: Direction) -> (KeyHalf, KeyHalf) {
        match direction {
            Direction::Bidirectional => (self.half(0), self.half(0)),
            Direction::Normal => (self.half(0), self.half(1)),
            Direction::Inverse => (self.half(1), self.half(0)),
        }
    }

    /// As metades que o cliente usa no `tls-crypt`: o cliente é a direção "inversa" (envia com a
    /// metade 1 e recebe com a 0), e o servidor a normal.
    ///
    /// Aberto (**confira**): a direção do cliente vem da descrição do protocolo e da memória
    /// da especificação; a prova é o teste contra um servidor OpenVPN de verdade.
    pub fn tls_crypt_halves(&self) -> (KeyHalf, KeyHalf) {
        (self.half(1), self.half(0))
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StaticKey {
        let mut bytes = [0u8; STATIC_KEY_LEN];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = i as u8;
        }
        StaticKey::from_bytes(bytes)
    }

    #[test]
    fn ida_e_volta_do_texto() {
        let key = sample();
        let text = key.to_pem();
        assert!(text.starts_with(BEGIN));
        assert_eq!(StaticKey::from_pem(&text).unwrap(), key);
    }

    #[test]
    fn ignora_o_que_esta_fora_das_marcas() {
        let key = sample();
        let text = format!(
            "#\n# 2048 bit OpenVPN static key\n#\n{}\nlixo depois\n",
            *key.to_pem()
        );
        assert_eq!(StaticKey::from_pem(&text).unwrap(), key);
    }

    #[test]
    fn aceita_maiusculas_e_final_de_linha_do_windows() {
        let key = sample();
        let text = key
            .to_pem()
            .to_uppercase()
            .replace("-----BEGIN OPENVPN STATIC KEY V1-----", BEGIN)
            .replace("-----END OPENVPN STATIC KEY V1-----", END)
            .replace('\n', "\r\n");
        assert_eq!(StaticKey::from_pem(&text).unwrap(), key);
    }

    #[test]
    fn erros_em_portugues() {
        assert_eq!(
            StaticKey::from_pem("nada"),
            Err(StaticKeyError::MissingMarkers)
        );
        let curta = format!("{BEGIN}\n00112233\n{END}\n");
        assert_eq!(
            StaticKey::from_pem(&curta),
            Err(StaticKeyError::WrongSize(4))
        );
        let ruim = format!("{BEGIN}\n00zz\n{END}\n");
        assert_eq!(
            StaticKey::from_pem(&ruim),
            Err(StaticKeyError::InvalidHex { line: 2 })
        );
        assert!(
            StaticKeyError::WrongSize(4)
                .to_string()
                .contains("256 bytes")
        );
    }

    #[test]
    fn as_metades_seguem_o_layout() {
        let key = sample();
        let h0 = key.half(0);
        let h1 = key.half(1);
        assert_eq!(h0.cipher[0], 0);
        assert_eq!(h0.hmac[0], 64);
        assert_eq!(h1.cipher[0], 128);
        assert_eq!(h1.hmac[0], 192);
    }

    #[test]
    fn direcoes_do_tls_auth() {
        let key = sample();
        let (out, inn) = key.tls_auth_halves(Direction::from_key_direction(Some(1)));
        assert_eq!(out.hmac[0], 192);
        assert_eq!(inn.hmac[0], 64);
        let (out, inn) = key.tls_auth_halves(Direction::from_key_direction(Some(0)));
        assert_eq!(out.hmac[0], 64);
        assert_eq!(inn.hmac[0], 192);
        let (out, inn) = key.tls_auth_halves(Direction::from_key_direction(None));
        assert_eq!(out.hmac[0], 64);
        assert_eq!(inn.hmac[0], 64);
    }

    #[test]
    fn o_debug_nao_mostra_a_chave() {
        let texto = format!("{:?}", sample());
        assert_eq!(texto, "StaticKey(..)");
    }
}
