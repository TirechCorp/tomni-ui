//! O leitor de Server-Sent Events, escrito à mão: junta as linhas `data:` de cada evento e
//! aceita `\n`, `\r\n` e `\r`, comentários, e eventos ou caracteres UTF-8 quebrados entre dois
//! blocos de leitura.

/// Um evento completo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SseEvent {
    /// O campo `event:`, se veio.
    pub name: Option<String>,
    /// As linhas `data:` juntadas com `\n`.
    pub data: String,
}

/// O leitor, alimentado com os blocos que chegam da rede.
#[derive(Default)]
pub(crate) struct SseParser {
    /// Os bytes da linha em andamento (podem acabar no meio de um caractere UTF-8).
    line: Vec<u8>,
    /// O último byte foi `\r`: um `\n` logo depois é a mesma quebra.
    after_cr: bool,
    name: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    /// Alimenta o leitor com o próximo bloco e devolve os eventos que ele completou.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        let mut out = Vec::new();
        for &byte in bytes {
            match byte {
                b'\n' if self.after_cr => self.after_cr = false,
                b'\n' | b'\r' => {
                    self.after_cr = byte == b'\r';
                    self.end_line(&mut out);
                }
                _ => {
                    self.after_cr = false;
                    self.line.push(byte);
                }
            }
        }
        out
    }

    /// O fim do fluxo: aproveita a linha e o evento que ficaram pela metade.
    pub(crate) fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if !self.line.is_empty() {
            self.end_line(&mut out);
        }
        self.dispatch(&mut out);
        out
    }

    fn end_line(&mut self, out: &mut Vec<SseEvent>) {
        let bytes = std::mem::take(&mut self.line);
        let line = String::from_utf8_lossy(&bytes);
        if line.is_empty() {
            self.dispatch(out);
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (&*line, ""),
        };
        match field {
            "data" => self.data.push(value.to_owned()),
            "event" => self.name = Some(value.to_owned()),
            // `id:`, `retry:` e o resto não servem aqui.
            _ => {}
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        let name = self.name.take();
        if self.data.is_empty() {
            return;
        }
        out.push(SseEvent {
            name,
            data: std::mem::take(&mut self.data).join("\n"),
        });
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn tudo(entrada: &[u8]) -> Vec<SseEvent> {
        let mut leitor = SseParser::default();
        let mut eventos = leitor.push(entrada);
        eventos.extend(leitor.finish());
        eventos
    }

    fn dados(eventos: &[SseEvent]) -> Vec<&str> {
        eventos.iter().map(|e| e.data.as_str()).collect()
    }

    #[test]
    fn sse_junta_data_de_varias_linhas() {
        let eventos = tudo(b"event: x\ndata: a\ndata: b\ndata:c\n\ndata: d\n\n");
        assert_eq!(dados(&eventos), ["a\nb\nc", "d"]);
        assert_eq!(eventos[0].name.as_deref(), Some("x"));
        assert_eq!(eventos[1].name, None);
    }

    #[test]
    fn sse_aceita_crlf_e_cr() {
        assert_eq!(dados(&tudo(b"data: a\r\n\r\ndata: b\r\n\r\n")), ["a", "b"]);
        assert_eq!(dados(&tudo(b"data: a\r\rdata: b\r\r")), ["a", "b"]);
        assert_eq!(dados(&tudo(b"data: a\n\ndata: b\n\n")), ["a", "b"]);
    }

    #[test]
    fn sse_ignora_comentarios() {
        let eventos = tudo(b": OPENROUTER PROCESSING\n\n:oi\ndata: a\nid: 7\nretry: 100\n\n");
        assert_eq!(dados(&eventos), ["a"]);
    }

    #[test]
    fn sse_evento_quebrado_entre_blocos() {
        let mut leitor = SseParser::default();
        assert!(leitor.push(b"da").is_empty());
        assert!(leitor.push(b"ta: meio").is_empty());
        assert!(leitor.push(b"\r").is_empty());
        // O `\n` do `\r\n` que chegou em outro bloco não vira linha vazia.
        assert!(leitor.push(b"\ndata: fim\r").is_empty());
        assert_eq!(dados(&leitor.push(b"\n\r\n")), ["meio\nfim"]);
    }

    #[test]
    fn sse_utf8_quebrado_entre_blocos() {
        let texto = "data: açaí 日本\n\n".as_bytes();
        for corte in 1..texto.len() {
            let mut leitor = SseParser::default();
            let mut eventos = leitor.push(&texto[..corte]);
            eventos.extend(leitor.push(&texto[corte..]));
            assert_eq!(dados(&eventos), ["açaí 日本"], "corte em {corte}");
        }
    }

    #[test]
    fn sse_aproveita_o_que_ficou_no_fim() {
        assert_eq!(dados(&tudo(b"data: a\n\ndata: b")), ["a", "b"]);
        assert_eq!(dados(&tudo(b"data: a\n\nevent: so-nome\n\n")), ["a"]);
    }
}
