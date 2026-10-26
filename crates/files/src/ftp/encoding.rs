//! A codificação dos nomes e das mensagens do FTP: UTF-8 ou Latin-1, feitas à mão.

use std::io;

/// Como os bytes dos comandos e das respostas viram texto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Charset {
    Utf8,
    Latin1,
}

impl Charset {
    /// Os bytes de `text` nesta codificação. `\r` e `\n` são recusados (injeção de comando) e,
    /// no Latin-1, também o caractere que não cabe nele.
    pub(crate) fn encode(self, text: &str) -> io::Result<Vec<u8>> {
        if text.contains(['\r', '\n']) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "o nome não pode ter quebra de linha no FTP",
            ));
        }
        match self {
            Charset::Utf8 => Ok(text.as_bytes().to_vec()),
            Charset::Latin1 => text
                .chars()
                .map(|c| {
                    u8::try_from(u32::from(c)).map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "o nome não cabe na codificação Latin-1 do servidor",
                        )
                    })
                })
                .collect(),
        }
    }

    /// O texto de `bytes` nesta codificação. O UTF-8 inválido vira `U+FFFD`.
    pub(crate) fn decode(self, bytes: &[u8]) -> String {
        match self {
            Charset::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
            Charset::Latin1 => bytes.iter().map(|&b| char::from(b)).collect(),
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn latin1_vai_e_volta() {
        let bytes = Charset::Latin1.encode("ação #1").unwrap();
        assert_eq!(bytes, b"a\xe7\xe3o #1");
        assert_eq!(Charset::Latin1.decode(&bytes), "ação #1");
    }

    #[test]
    fn latin1_recusa_o_que_nao_cabe() {
        let erro = Charset::Latin1.encode("日本").unwrap_err();
        assert!(erro.to_string().contains("Latin-1"));
    }

    #[test]
    fn quebra_de_linha_e_recusada() {
        assert!(Charset::Utf8.encode("a\r\nDELE b").is_err());
        assert!(Charset::Latin1.encode("a\nb").is_err());
    }

    #[test]
    fn utf8_passa_direto() {
        assert_eq!(Charset::Utf8.encode("não").unwrap(), "não".as_bytes());
        assert_eq!(Charset::Utf8.decode("não".as_bytes()), "não");
    }
}
