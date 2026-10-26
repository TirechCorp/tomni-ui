//! Pequenas funções de apoio: hexadecimal, o relógio e datas para as mensagens.

use std::time::{SystemTime, UNIX_EPOCH};

/// Os bytes em hexadecimal minúsculo (a forma das impressões digitais do Nebula).
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITOS: &[u8; 16] = b"0123456789abcdef";
    let mut texto = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        texto.push(char::from(DIGITOS[usize::from(byte >> 4)]));
        texto.push(char::from(DIGITOS[usize::from(byte & 0x0f)]));
    }
    texto
}

/// O inverso de [`hex`]; `None` se o texto tem tamanho ímpar ou algo que não é dígito.
#[cfg(test)]
pub(crate) fn unhex(texto: &str) -> Option<Vec<u8>> {
    let texto = texto.trim().as_bytes();
    if !texto.len().is_multiple_of(2) {
        return None;
    }
    texto
        .chunks(2)
        .map(|par| {
            let alto = char::from(par[0]).to_digit(16)?;
            let baixo = char::from(par[1]).to_digit(16)?;
            u8::try_from(alto * 16 + baixo).ok()
        })
        .collect()
}

/// Os segundos desde 1970 (o relógio do computador).
pub(crate) fn now_unix() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(depois) => i64::try_from(depois.as_secs()).unwrap_or(i64::MAX),
        Err(antes) => -i64::try_from(antes.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

/// A data (`dd/mm/aaaa`, no UTC) de um instante em segundos desde 1970, para as mensagens.
pub fn format_date(unix: i64) -> String {
    // Dias desde 1970 → data civil (algoritmo de Howard Hinnant, `civil_from_days`).
    let dias = unix.div_euclid(86_400);
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dia = doy - (153 * mp + 2) / 5 + 1;
    let mes = if mp < 10 { mp + 3 } else { mp - 9 };
    let ano = yoe + era * 400 + i64::from(mes <= 2);
    format!("{dia:02}/{mes:02}/{ano}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hexadecimal_vai_e_volta() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(unhex("00ABff"), Some(vec![0x00, 0xab, 0xff]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }

    #[test]
    fn datas_conferem_com_valores_conhecidos() {
        assert_eq!(format_date(0), "01/01/1970");
        assert_eq!(format_date(951_782_400), "29/02/2000");
        assert_eq!(format_date(1_792_641_528), "22/10/2026");
        assert_eq!(format_date(-86_400), "31/12/1969");
    }
}
