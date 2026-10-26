//! Protobuf escrito à mão: só o que o Nebula usa (varint, campos de bytes, inteiros e listas
//! empacotadas). A assinatura do certificado v1 cobre a codificação canônica dos detalhes,
//! então a ordem dos campos e a omissão dos valores padrão (proto3) precisam ser exatas.

/// Escreve uma mensagem. Os campos saem na ordem das chamadas; quem chama as faz em ordem
/// crescente de número, como o Go.
#[derive(Default)]
pub(crate) struct Writer {
    buf: Vec<u8>,
}

const WIRE_VARINT: u8 = 0;
const WIRE_BYTES: u8 = 2;

impl Writer {
    pub(crate) fn new() -> Writer {
        Writer::default()
    }

    fn varint(&mut self, mut valor: u64) {
        while valor >= 0x80 {
            self.buf.push((valor as u8) | 0x80);
            valor >>= 7;
        }
        self.buf.push(valor as u8);
    }

    fn tag(&mut self, campo: u32, tipo: u8) {
        self.varint(u64::from(campo) << 3 | u64::from(tipo));
    }

    /// Um campo `bytes` ou `string`, omitido se vazio (o padrão do proto3).
    pub(crate) fn bytes(&mut self, campo: u32, valor: &[u8]) {
        if !valor.is_empty() {
            self.bytes_always(campo, valor);
        }
    }

    /// Um campo `bytes`, `string` ou mensagem que sai mesmo vazio (elemento de lista, submensagem).
    pub(crate) fn bytes_always(&mut self, campo: u32, valor: &[u8]) {
        self.tag(campo, WIRE_BYTES);
        self.varint(valor.len() as u64);
        self.buf.extend_from_slice(valor);
    }

    /// Um inteiro sem sinal, omitido se zero.
    pub(crate) fn uint(&mut self, campo: u32, valor: u64) {
        if valor != 0 {
            self.tag(campo, WIRE_VARINT);
            self.varint(valor);
        }
    }

    /// Um `int64`, omitido se zero (o negativo ocupa os 10 bytes do complemento de dois).
    pub(crate) fn int(&mut self, campo: u32, valor: i64) {
        self.uint(campo, valor as u64);
    }

    /// Um `bool`, omitido se falso.
    pub(crate) fn boolean(&mut self, campo: u32, valor: bool) {
        self.uint(campo, u64::from(valor));
    }

    /// Uma lista de `uint32` empacotada, omitida se vazia.
    pub(crate) fn packed_u32(&mut self, campo: u32, valores: &[u32]) {
        if valores.is_empty() {
            return;
        }
        let mut interno = Writer::new();
        for valor in valores {
            interno.varint(u64::from(*valor));
        }
        self.bytes_always(campo, &interno.buf);
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// O valor de um campo lido.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    /// Um campo de 32 ou 64 bits fixos, que o Nebula não usa: lido e ignorado.
    Fixed,
}

impl<'a> Value<'a> {
    pub(crate) fn bytes(self) -> Option<&'a [u8]> {
        match self {
            Value::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub(crate) fn varint(self) -> Option<u64> {
        match self {
            Value::Varint(valor) => Some(valor),
            _ => None,
        }
    }
}

/// Lê os campos de uma mensagem, um a um. Os campos desconhecidos vêm como qualquer outro (quem
/// lê os ignora); só uma mensagem malformada dá erro.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut valor = 0u64;
        for (indice, byte) in self.data.iter().enumerate() {
            if indice >= 10 {
                return None;
            }
            valor |= u64::from(byte & 0x7f) << (7 * indice);
            if byte & 0x80 == 0 {
                self.data = &self.data[indice + 1..];
                return Some(valor);
            }
        }
        None
    }
}

impl<'a> Iterator for Reader<'a> {
    /// `Err(())` é uma mensagem malformada.
    type Item = Result<(u32, Value<'a>), ()>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.data.is_empty() {
            return None;
        }
        let lido = (|| {
            let tag = self.varint()?;
            let campo = u32::try_from(tag >> 3).ok().filter(|campo| *campo != 0)?;
            let valor = match tag & 7 {
                0 => Value::Varint(self.varint()?),
                1 => {
                    self.data = self.data.get(8..)?;
                    Value::Fixed
                }
                2 => {
                    let tamanho = usize::try_from(self.varint()?).ok()?;
                    let bytes = self.data.get(..tamanho)?;
                    self.data = &self.data[tamanho..];
                    Value::Bytes(bytes)
                }
                5 => {
                    self.data = self.data.get(4..)?;
                    Value::Fixed
                }
                _ => return None,
            };
            Some((campo, valor))
        })();
        match lido {
            Some(par) => Some(Ok(par)),
            None => {
                self.data = &[];
                Some(Err(()))
            }
        }
    }
}

/// Os `uint32` de um campo repetido: empacotado (bytes) ou um por campo (varint).
pub(crate) fn push_u32s(destino: &mut Vec<u32>, valor: Value<'_>) -> Result<(), ()> {
    match valor {
        Value::Varint(valor) => destino.push(u32::try_from(valor).map_err(|_| ())?),
        Value::Bytes(bytes) => {
            let mut leitor = Reader { data: bytes };
            while !leitor.data.is_empty() {
                let valor = leitor.varint().ok_or(())?;
                destino.push(u32::try_from(valor).map_err(|_| ())?);
            }
        }
        Value::Fixed => return Err(()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_nos_limites() {
        for valor in [0u64, 1, 127, 128, 300, u64::from(u32::MAX), u64::MAX] {
            let mut w = Writer::new();
            w.tag(1, WIRE_VARINT);
            w.varint(valor);
            let bytes = w.finish();
            let campos: Vec<_> = Reader::new(&bytes).collect();
            assert_eq!(campos, [Ok((1, Value::Varint(valor)))]);
        }
        // O maior varint tem 10 bytes; 11 é malformado.
        let longo = [
            0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01,
        ];
        assert_eq!(Reader::new(&longo).next(), Some(Err(())));
    }

    #[test]
    fn omite_os_valores_padrao_e_mantem_a_ordem() {
        let mut w = Writer::new();
        w.bytes(1, b"");
        w.uint(2, 0);
        w.boolean(3, false);
        w.packed_u32(4, &[]);
        assert!(w.finish().is_empty());

        let mut w = Writer::new();
        w.bytes(1, b"no");
        w.packed_u32(2, &[1, 300]);
        w.int(5, -1);
        w.uint(100, 1);
        let bytes = w.finish();
        assert_eq!(&bytes[..4], [0x0a, 2, b'n', b'o']);
        let campos: Vec<_> = Reader::new(&bytes).map(Result::unwrap).collect();
        assert_eq!(campos[0], (1, Value::Bytes(b"no")));
        assert_eq!(campos[2], (5, Value::Varint(u64::MAX)));
        assert_eq!(campos[3], (100, Value::Varint(1)));
        let mut lista = Vec::new();
        push_u32s(&mut lista, campos[1].1).unwrap();
        assert_eq!(lista, [1, 300]);
    }

    #[test]
    fn campo_desconhecido_e_pulado_e_truncado_e_erro() {
        let mut w = Writer::new();
        w.uint(50, 7);
        w.bytes(51, b"x");
        w.uint(1, 9);
        let mut bytes = w.finish();
        // Um campo de 32 bits fixos (tipo 5) no meio.
        bytes.extend_from_slice(&[0x0d, 1, 2, 3, 4]);
        let campos: Vec<_> = Reader::new(&bytes).map(Result::unwrap).collect();
        assert_eq!(campos.len(), 4);
        assert_eq!(campos[3].1, Value::Fixed);
        // Um bytes que diz ter mais do que existe.
        assert_eq!(Reader::new(&[0x0a, 5, 1]).next(), Some(Err(())));
        // O tipo 3 (grupo) não existe no proto3.
        assert_eq!(Reader::new(&[0x0b]).next(), Some(Err(())));
    }
}
