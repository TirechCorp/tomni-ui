//! O filtro do [`LogMode::Text`](crate::LogMode::Text): tira as sequências de controle da saída
//! do programa e deixa o texto que se lê na tela.

use vte::{Parser, Perform};

/// Passa a saída do programa por um `vte::Perform` que guarda só o imprimível, o `\n` e o `\t`
/// (o `\r`, o backspace e o resto dos controles caem). Lembra o estado entre as chamadas: uma
/// sequência cortada no meio de um bloco termina no seguinte.
#[derive(Default)]
pub(crate) struct LogFilter {
    parser: Parser,
}

/// O texto limpo de um bloco.
#[derive(Default)]
struct Text {
    out: Vec<u8>,
}

impl Perform for Text {
    fn print(&mut self, c: char) {
        // Os controles C1 (U+0080 a U+009F) que chegam como UTF-8 passam como imprimíveis pelo
        // `vte`; num texto para ler (ou para mandar a outro programa) eles não servem.
        if c.is_control() {
            return;
        }
        let mut utf8 = [0; 4];
        self.out
            .extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
    }

    fn execute(&mut self, byte: u8) {
        if matches!(byte, b'\n' | b'\t') {
            self.out.push(byte);
        }
    }
}

impl LogFilter {
    /// Os bytes de `bytes` sem as sequências de controle.
    pub(crate) fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut text = Text::default();
        self.parser.advance(&mut text, bytes);
        text.out
    }
}

/// O texto de `bytes` sem as sequências de controle (CSI, OSC, DCS, APC, PM, SOS) e sem os
/// caracteres de controle, exceto `\n` e `\t`: o mesmo que o log em
/// [`LogMode::Text`](crate::LogMode::Text) grava, incluindo os controles C1. Bytes que não são
/// UTF-8 válido viram `U+FFFD`.
///
/// ```
/// use tomni_ui_terminal::plain_text;
///
/// let tela = b"\x1b[1;32mok\x1b[0m\r\n\x1b]0;titulo\x07fim";
/// assert_eq!(plain_text(tela), "ok\nfim");
/// ```
pub fn plain_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&LogFilter::default().filter(bytes)).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fica_so_o_texto_com_quebras_e_tabs() {
        let mut filtro = LogFilter::default();
        let saida =
            filtro.filter(b"\x1b[1;32mol\xc3\xa1\x1b[0m\r\n\tfim\x07\x08\x1b]0;titulo\x07!");
        assert_eq!(String::from_utf8(saida).unwrap(), "olá\n\tfim!");
    }

    #[test]
    fn sequencia_cortada_termina_no_bloco_seguinte() {
        let mut filtro = LogFilter::default();
        let mut saida = filtro.filter(b"a\x1b[3");
        saida.extend(filtro.filter(b"1mb"));
        assert_eq!(saida, b"ab");
    }

    #[test]
    fn plain_text_tira_csi_osc_e_c1() {
        // CSI, OSC com BEL e com ST, DCS, APC e um C1 que chega como UTF-8 (U+009B).
        let entrada = "a\x1b[31mb\x1b[0m\x1b]0;t\x07c\x1b]8;;http://x\x1b\\d\x1bPq#0\x1b\\e\x1b_app\x1b\\f\u{9b}31mg\u{85}h\r\n\ti";
        assert_eq!(plain_text(entrada.as_bytes()), "abcdef31mgh\n\ti");
        assert_eq!(plain_text(b""), "");
        assert_eq!(plain_text(b"\xff!"), "\u{fffd}!");
    }

    #[test]
    fn plain_text_com_uma_gravacao_de_verdade() {
        let gravacao = b"\x1b[?2004h\x1b]0;user@host: ~\x07\x1b[01;32muser@host\x1b[00m:\x1b[01;34m~\x1b[00m$ ls\r\n\x1b[0m\x1b[01;34mdocs\x1b[0m  notas.txt\r\n";
        assert_eq!(plain_text(gravacao), "user@host:~$ ls\ndocs  notas.txt\n");
    }
}
