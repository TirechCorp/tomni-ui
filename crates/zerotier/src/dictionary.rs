//! O dicionário de chave=valor em texto que o controlador usa na configuração da rede
//! (`Dictionary.hpp`): uma linha por entrada, `chave=valor`, com o valor escapado para nunca ter
//! quebra de linha, `=` nem zero solto. A forma serializada termina no primeiro zero, como uma
//! string de C, então o que vem depois dele não faz parte do dicionário.
//!
//! O dicionário original tem capacidade fixa; este não: quem recebe a configuração já limita o
//! tamanho pelo que o protocolo permite remontar.

/// Um dicionário na ordem em que as entradas foram escritas.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dictionary {
    entradas: Vec<(Vec<u8>, Vec<u8>)>,
}

impl Dictionary {
    /// Um dicionário vazio.
    pub fn new() -> Self {
        Self::default()
    }

    /// Lê a forma serializada. Linhas sem `=` são ignoradas, como no original.
    pub fn parse(bytes: &[u8]) -> Self {
        let fim = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        let mut entradas = Vec::new();
        for linha in bytes[..fim].split(|&b| b == b'\n' || b == b'\r') {
            let Some(igual) = linha.iter().position(|&b| b == b'=') else {
                continue;
            };
            entradas.push((linha[..igual].to_vec(), desfazer(&linha[igual + 1..])));
        }
        Self { entradas }
    }

    /// Acrescenta uma entrada (se a chave já existe, `get` continua devolvendo a primeira).
    /// A chave não pode ter `=`, quebra de linha nem zero.
    pub fn add(&mut self, chave: &str, valor: &[u8]) {
        debug_assert!(!chave.bytes().any(|b| matches!(b, 0 | b'\r' | b'\n' | b'=')));
        self.entradas
            .push((chave.as_bytes().to_vec(), valor.to_vec()));
    }

    /// Acrescenta um texto.
    pub fn add_str(&mut self, chave: &str, valor: &str) {
        self.add(chave, valor.as_bytes());
    }

    /// Acrescenta um inteiro em hexadecimal sem zeros à esquerda (o formato do original).
    pub fn add_u64(&mut self, chave: &str, valor: u64) {
        self.add_str(chave, &format!("{valor:x}"));
    }

    /// Acrescenta um booleano como `1` ou `0`.
    pub fn add_bool(&mut self, chave: &str, valor: bool) {
        self.add_str(chave, if valor { "1" } else { "0" });
    }

    /// O valor da primeira entrada com essa chave.
    pub fn get(&self, chave: &str) -> Option<&[u8]> {
        self.entradas
            .iter()
            .find(|(k, _)| k == chave.as_bytes())
            .map(|(_, v)| v.as_slice())
    }

    /// O valor como texto, se for UTF-8 válido.
    pub fn get_str(&self, chave: &str) -> Option<&str> {
        std::str::from_utf8(self.get(chave)?).ok()
    }

    /// O valor como inteiro em hexadecimal (`None` se faltar ou não for hexadecimal).
    pub fn get_u64(&self, chave: &str) -> Option<u64> {
        u64::from_str_radix(self.get_str(chave)?, 16).ok()
    }

    /// O valor como booleano: `1`, `t` ou `T` no começo é verdadeiro (`getB`).
    pub fn get_bool(&self, chave: &str) -> Option<bool> {
        let valor = self.get(chave)?;
        Some(matches!(valor.first(), Some(b'1' | b't' | b'T')))
    }

    /// Quantas entradas há.
    pub fn len(&self) -> usize {
        self.entradas.len()
    }

    /// Não há nenhuma?
    pub fn is_empty(&self) -> bool {
        self.entradas.is_empty()
    }

    /// A forma serializada, sem o zero final.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut saida = Vec::new();
        for (i, (chave, valor)) in self.entradas.iter().enumerate() {
            if i > 0 {
                saida.push(b'\n');
            }
            saida.extend_from_slice(chave);
            saida.push(b'=');
            for &b in valor {
                match b {
                    0 => saida.extend_from_slice(b"\\0"),
                    b'\r' => saida.extend_from_slice(b"\\r"),
                    b'\n' => saida.extend_from_slice(b"\\n"),
                    b'\\' => saida.extend_from_slice(b"\\\\"),
                    b'=' => saida.extend_from_slice(b"\\e"),
                    outro => saida.push(outro),
                }
            }
        }
        saida
    }
}

/// Desfaz o escape do valor. Uma barra no fim da linha vale por ela mesma ignorada.
fn desfazer(valor: &[u8]) -> Vec<u8> {
    let mut saida = Vec::with_capacity(valor.len());
    let mut escapado = false;
    for &b in valor {
        if escapado {
            escapado = false;
            saida.push(match b {
                b'r' => b'\r',
                b'n' => b'\n',
                b'0' => 0,
                b'e' => b'=',
                outro => outro,
            });
        } else if b == b'\\' {
            escapado = true;
        } else {
            saida.push(b);
        }
    }
    saida
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn escreve_e_le_com_os_escapes() {
        let mut d = Dictionary::new();
        d.add_str("nome", "rede=de\ntestes\\fim");
        d.add("binario", &[0, 1, b'\r', 255]);
        d.add_u64("id", 0x8056_c2e2_1c00_0001);
        d.add_bool("ativa", true);
        let bytes = d.to_bytes();
        assert!(!bytes.contains(&0));
        assert_eq!(bytes.iter().filter(|&&b| b == b'\n').count(), 3);
        let lido = Dictionary::parse(&bytes);
        assert_eq!(lido, d);
        assert_eq!(lido.get_str("nome"), Some("rede=de\ntestes\\fim"));
        assert_eq!(lido.get("binario"), Some(&[0, 1, b'\r', 255][..]));
        assert_eq!(lido.get_u64("id"), Some(0x8056_c2e2_1c00_0001));
        assert_eq!(lido.get_bool("ativa"), Some(true));
    }

    #[test]
    fn a_primeira_chave_repetida_vence_e_o_que_falta_e_none() {
        let d = Dictionary::parse(b"a=1\nb=2\na=3");
        assert_eq!(d.get_str("a"), Some("1"));
        assert_eq!(d.get_str("c"), None);
        assert_eq!(d.get_u64("c"), None);
        assert_eq!(d.get_bool("c"), None);
    }

    #[test]
    fn para_no_primeiro_zero_aceita_cr_lf_e_ignora_linha_sem_igual() {
        let d = Dictionary::parse(b"a=1\r\nlixo\r\nb=2\0c=3");
        assert_eq!(d.len(), 2);
        assert_eq!(d.get_str("b"), Some("2"));
        assert_eq!(d.get_str("c"), None);
    }

    #[test]
    fn booleano_e_hexadecimal_seguem_o_original() {
        let d = Dictionary::parse(b"t=true\nf=0\nh=ff\nruim=zz");
        assert_eq!(d.get_bool("t"), Some(true));
        assert_eq!(d.get_bool("f"), Some(false));
        assert_eq!(d.get_u64("h"), Some(255));
        assert_eq!(d.get_u64("ruim"), None);
    }
}
