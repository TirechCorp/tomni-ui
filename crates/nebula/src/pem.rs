//! O PEM dos certificados e das chaves do Nebula: `-----BEGIN NEBULA CERTIFICATE-----`, o
//! conteúdo em base64 e `-----END …-----`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Um bloco PEM: o rótulo do banner e os bytes decodificados.
#[derive(Debug)]
pub(crate) struct Block {
    pub(crate) label: String,
    pub(crate) data: Vec<u8>,
}

const BEGIN: &str = "-----BEGIN ";
const END: &str = "-----END ";
const TRAIL: &str = "-----";

/// Lê todos os blocos de um texto. O que está fora dos blocos (comentários, linhas em branco) é
/// ignorado.
pub(crate) fn decode(text: &str) -> Result<Vec<Block>, String> {
    let mut blocks = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        let line = line.trim();
        match &mut current {
            None => {
                if let Some(label) = line
                    .strip_prefix(BEGIN)
                    .and_then(|rest| rest.strip_suffix(TRAIL))
                {
                    current = Some((label.to_owned(), String::new()));
                }
            }
            Some((label, body)) => {
                if let Some(end) = line
                    .strip_prefix(END)
                    .and_then(|rest| rest.strip_suffix(TRAIL))
                {
                    if end != label {
                        return Err(format!(
                            "o bloco PEM {label} termina como {end}, que é outro"
                        ));
                    }
                    let data = STANDARD
                        .decode(body.as_bytes())
                        .map_err(|_| format!("o conteúdo do bloco PEM {label} não é base64"))?;
                    blocks.push(Block {
                        label: std::mem::take(label),
                        data,
                    });
                    current = None;
                } else {
                    body.push_str(line);
                }
            }
        }
    }
    if let Some((label, _)) = current {
        return Err(format!("o bloco PEM {label} não tem a linha de fim"));
    }
    Ok(blocks)
}

/// Escreve um bloco PEM com linhas de 64 colunas.
pub(crate) fn encode(label: &str, data: &[u8]) -> String {
    let body = STANDARD.encode(data);
    let mut text = format!("{BEGIN}{label}{TRAIL}\n");
    for line in body.as_bytes().chunks(64) {
        // O base64 é ASCII, então a fatia é sempre UTF-8 válido.
        text.push_str(std::str::from_utf8(line).unwrap_or_default());
        text.push('\n');
    }
    text.push_str(&format!("{END}{label}{TRAIL}\n"));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escreve_e_le_varios_blocos_ignorando_o_resto() {
        let texto = format!(
            "# comentário\n{}\nlixo\n{}",
            encode("A", &[1, 2, 3]),
            encode("NEBULA CERTIFICATE", &[7; 100])
        );
        let blocos = decode(&texto).unwrap();
        assert_eq!(blocos.len(), 2);
        assert_eq!(blocos[0].label, "A");
        assert_eq!(blocos[0].data, [1, 2, 3]);
        assert_eq!(blocos[1].data.len(), 100);
    }

    #[test]
    fn bloco_sem_fim_ou_com_base64_ruim_e_recusado() {
        assert!(decode("-----BEGIN A-----\nAQID\n").is_err());
        assert!(decode("-----BEGIN A-----\n!!!\n-----END A-----\n").is_err());
        assert!(decode("-----BEGIN A-----\nAQID\n-----END B-----\n").is_err());
    }
}
