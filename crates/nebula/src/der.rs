//! DER escrito à mão: só o que o certificado v2 usa (TLV, `SEQUENCE`, `OCTET STRING`, inteiros
//! pequenos e tags de contexto). Lê-se à mão para não depender de como uma biblioteca trata o
//! `DEFAULT` e as tags implícitas do `AUTOMATIC TAGS`.

pub(crate) const SEQUENCE: u8 = 0x30;
pub(crate) const OCTET_STRING: u8 = 0x04;
pub(crate) const UTF8_STRING: u8 = 0x0c;

/// Lê elementos TLV de uma sequência de bytes.
#[derive(Clone, Copy)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// A tag do próximo elemento, sem consumi-lo.
    pub(crate) fn peek_tag(&self) -> Option<u8> {
        self.data.first().copied()
    }

    /// Consome o próximo elemento inteiro (tag, tamanho e conteúdo) e devolve o conteúdo.
    fn element(&mut self) -> Option<(u8, &'a [u8], &'a [u8])> {
        let tag = *self.data.first()?;
        // Tags com número alto (0x1f) não existem no Nebula.
        if tag & 0x1f == 0x1f {
            return None;
        }
        let primeiro = *self.data.get(1)?;
        let (tamanho, cabecalho) = if primeiro < 0x80 {
            (usize::from(primeiro), 2)
        } else {
            let bytes = usize::from(primeiro & 0x7f);
            if bytes == 0 || bytes > 4 {
                return None;
            }
            let mut tamanho = 0usize;
            for byte in self.data.get(2..2 + bytes)? {
                tamanho = tamanho << 8 | usize::from(*byte);
            }
            (tamanho, 2 + bytes)
        };
        let fim = cabecalho.checked_add(tamanho)?;
        let inteiro = self.data.get(..fim)?;
        let conteudo = &inteiro[cabecalho..];
        self.data = &self.data[fim..];
        Some((tag, inteiro, conteudo))
    }

    /// O conteúdo do próximo elemento, que precisa ter a tag `tag`.
    pub(crate) fn read(&mut self, tag: u8) -> Option<&'a [u8]> {
        if self.peek_tag()? != tag {
            return None;
        }
        self.element().map(|(_, _, conteudo)| conteudo)
    }

    /// O próximo elemento **inteiro** (com tag e tamanho), que precisa ter a tag `tag`: a
    /// assinatura cobre os detalhes exatamente como vieram.
    pub(crate) fn read_element(&mut self, tag: u8) -> Option<&'a [u8]> {
        if self.peek_tag()? != tag {
            return None;
        }
        self.element().map(|(_, inteiro, _)| inteiro)
    }

    /// Se o próximo elemento tem a tag `tag`, consome e devolve o conteúdo; senão `Ok(None)` e
    /// nada é consumido. `Err(())` se o elemento é malformado.
    pub(crate) fn read_optional(&mut self, tag: u8) -> Result<Option<&'a [u8]>, ()> {
        if self.peek_tag() != Some(tag) {
            return Ok(None);
        }
        self.element()
            .map(|(_, _, conteudo)| Some(conteudo))
            .ok_or(())
    }
}

/// Escreve um elemento TLV: a tag, o tamanho (forma curta ou longa) e o conteúdo.
pub(crate) fn tlv(tag: u8, conteudo: &[u8]) -> Vec<u8> {
    let mut saida = Vec::with_capacity(conteudo.len() + 6);
    saida.push(tag);
    let tamanho = conteudo.len();
    if tamanho < 0x80 {
        saida.push(tamanho as u8);
    } else {
        let bytes: Vec<u8> = tamanho
            .to_be_bytes()
            .into_iter()
            .skip_while(|byte| *byte == 0)
            .collect();
        saida.push(0x80 | bytes.len() as u8);
        saida.extend_from_slice(&bytes);
    }
    saida.extend_from_slice(conteudo);
    saida
}

/// O conteúdo de um `INTEGER` com sinal, no menor número de bytes (complemento de dois).
#[cfg(test)]
pub(crate) fn int_content(valor: i64) -> Vec<u8> {
    let bytes = valor.to_be_bytes();
    let mut inicio = 0;
    while inicio < 7 {
        let atual = bytes[inicio];
        let proximo = bytes[inicio + 1];
        // Descarta o byte se ele só repete o sinal do seguinte.
        if (atual == 0x00 && proximo & 0x80 == 0) || (atual == 0xff && proximo & 0x80 != 0) {
            inicio += 1;
        } else {
            break;
        }
    }
    bytes[inicio..].to_vec()
}

/// Lê o conteúdo de um `INTEGER` com sinal de até 8 bytes.
pub(crate) fn parse_int(conteudo: &[u8]) -> Option<i64> {
    if conteudo.is_empty() || conteudo.len() > 8 {
        return None;
    }
    // O DER exige o menor tamanho: um primeiro byte que só repete o sinal do seguinte sobra.
    if conteudo.len() > 1
        && ((conteudo[0] == 0x00 && conteudo[1] & 0x80 == 0)
            || (conteudo[0] == 0xff && conteudo[1] & 0x80 != 0))
    {
        return None;
    }
    let mut valor: i64 = if conteudo[0] & 0x80 != 0 { -1 } else { 0 };
    for byte in conteudo {
        valor = valor << 8 | i64::from(*byte);
    }
    Some(valor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tamanhos_curto_e_longo_vao_e_voltam() {
        for tamanho in [0usize, 1, 127, 128, 255, 256, 70_000] {
            let conteudo = vec![0xaa; tamanho];
            let bytes = tlv(OCTET_STRING, &conteudo);
            let mut leitor = Reader::new(&bytes);
            assert_eq!(leitor.read(OCTET_STRING), Some(&conteudo[..]));
            assert!(leitor.is_empty());
        }
        assert_eq!(tlv(0x04, &[1, 2]), [0x04, 2, 1, 2]);
        assert_eq!(&tlv(0x04, &[0; 128])[..3], [0x04, 0x81, 128]);
    }

    #[test]
    fn inteiros_no_menor_tamanho() {
        for (valor, bytes) in [
            (0i64, vec![0x00]),
            (127, vec![0x7f]),
            (128, vec![0x00, 0x80]),
            (-1, vec![0xff]),
            (-128, vec![0x80]),
            (-129, vec![0xff, 0x7f]),
            (1_792_641_528, vec![0x6a, 0xd9, 0x89, 0xf8]),
        ] {
            assert_eq!(int_content(valor), bytes, "{valor}");
            assert_eq!(parse_int(&bytes), Some(valor));
        }
        assert_eq!(parse_int(&[]), None);
        assert_eq!(parse_int(&[0; 9]), None);
        // Fora do menor tamanho.
        assert_eq!(parse_int(&[0x00, 0x01]), None);
        assert_eq!(parse_int(&[0xff, 0xff]), None);
    }

    #[test]
    fn leitura_recusa_o_malformado_e_devolve_o_elemento_inteiro() {
        assert!(Reader::new(&[0x04, 5, 1]).read(0x04).is_none());
        assert!(Reader::new(&[0x1f, 1, 0]).read(0x1f).is_none());
        assert!(Reader::new(&[0x04, 0x80]).read(0x04).is_none());
        let bytes = [0xa0, 2, 0x80, 0, 0x81, 1, 7];
        let mut leitor = Reader::new(&bytes);
        assert_eq!(leitor.read_element(0xa0), Some(&bytes[..4]));
        assert_eq!(leitor.read_optional(0x82), Ok(None));
        assert_eq!(leitor.read_optional(0x81), Ok(Some(&[7][..])));
    }
}
