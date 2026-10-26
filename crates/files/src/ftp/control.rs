//! A conexão de controle: comandos e respostas (RFC 959 §4.2, com as de várias linhas).

use std::fmt;
use std::io::{self, Read, Write};

use super::encoding::Charset;
use super::tls::Stream;

/// O maior tamanho aceito para uma linha de resposta.
const MAX_LINE: usize = 64 * 1024;

/// Uma resposta do servidor.
#[derive(Clone, Debug)]
pub(crate) struct Reply {
    pub code: u16,
    /// Todas as linhas, como vieram (sem o fim de linha).
    pub lines: Vec<String>,
}

impl Reply {
    /// O texto da resposta, sem o código: o da última linha ou, se vazio, o da primeira.
    pub(crate) fn text(&self) -> String {
        let strip = |line: &String| {
            let rest = line.get(3..).unwrap_or_default();
            rest.trim_start_matches(['-', ' ']).trim().to_owned()
        };
        let last = self.lines.last().map(strip).unwrap_or_default();
        if last.is_empty() {
            self.lines.first().map(strip).unwrap_or_default()
        } else {
            last
        }
    }

    /// `true` nas respostas de sucesso (`2xx`).
    pub(crate) fn ok(&self) -> bool {
        (200..300).contains(&self.code)
    }

    /// `true` nas respostas preliminares (`1xx`).
    pub(crate) fn preliminary(&self) -> bool {
        (100..200).contains(&self.code)
    }

    /// As linhas do meio de uma resposta de várias linhas (sem a primeira e a última).
    pub(crate) fn body(&self) -> &[String] {
        if self.lines.len() > 2 {
            &self.lines[1..self.lines.len() - 1]
        } else {
            &[]
        }
    }
}

/// A resposta de erro de um comando, para quem quer o código (o `LIST -a`, o fallback do TLS).
#[derive(Debug)]
pub(crate) struct FtpReply {
    pub code: u16,
    pub message: String,
}

impl fmt::Display for FtpReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FtpReply {}

/// O código da resposta que causou `error`, se foi uma.
pub(crate) fn reply_code(error: &io::Error) -> Option<u16> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<FtpReply>())
        .map(|reply| reply.code)
}

/// O erro de um comando que o servidor recusou. `action` e `path` dizem o que se tentava.
pub(crate) fn reply_error(reply: &Reply, action: &str, path: &str) -> io::Error {
    let text = reply.text();
    let lower = text.to_lowercase();
    let missing = [
        "no such",
        "not found",
        "does not exist",
        "não encontrado",
        "cannot find",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    let kind = match reply.code {
        421 => io::ErrorKind::ConnectionReset,
        530 | 532 | 553 => io::ErrorKind::PermissionDenied,
        550 | 450 if missing => io::ErrorKind::NotFound,
        550 => io::ErrorKind::PermissionDenied,
        502 | 504 => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::Other,
    };
    let detail = if reply.code == 421 {
        format!("o servidor encerrou a conexão: {text}")
    } else {
        format!("o servidor recusou: {text}")
    };
    let message = if path.is_empty() {
        detail
    } else {
        describe_message(kind, &detail, action, path)
    };
    io::Error::new(
        kind,
        FtpReply {
            code: reply.code,
            message,
        },
    )
}

fn describe_message(kind: io::ErrorKind, detail: &str, action: &str, path: &str) -> String {
    crate::describe(io::Error::new(kind, detail), action, path).to_string()
}

/// A conexão de controle: o fluxo, com TLS ou não, e o que sobrou da leitura.
pub(crate) struct Control {
    pub stream: Stream,
    buffer: Vec<u8>,
    pub charset: Charset,
    /// Respostas já lidas que ainda ninguém pediu (uma que chegou antes da hora).
    stashed: std::collections::VecDeque<Reply>,
}

impl Control {
    pub(crate) fn new(stream: Stream, charset: Charset) -> Self {
        Self {
            stream,
            buffer: Vec::new(),
            charset,
            stashed: std::collections::VecDeque::new(),
        }
    }

    /// Guarda uma resposta lida antes da hora: a próxima leitura a devolve primeiro.
    pub(crate) fn stash(&mut self, reply: Reply) {
        self.stashed.push_back(reply);
    }

    /// Manda um comando (`linha` sem o fim de linha).
    pub(crate) fn send(&mut self, line: &str) -> io::Result<()> {
        let mut bytes = self.charset.encode(line)?;
        bytes.extend_from_slice(b"\r\n");
        self.stream.write_all(&bytes)?;
        self.stream.flush()
    }

    /// Manda um comando e lê a resposta definitiva (as `1xx` são devolvidas, quem chama decide).
    pub(crate) fn command(&mut self, line: &str) -> io::Result<Reply> {
        self.send(line)?;
        self.read_reply()
    }

    fn read_line(&mut self) -> io::Result<Vec<u8>> {
        loop {
            if let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.buffer.drain(..=end).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(line);
            }
            if self.buffer.len() > MAX_LINE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "o servidor mandou uma resposta longa demais",
                ));
            }
            let mut chunk = [0u8; 4096];
            let read = match self.stream.read(&mut chunk) {
                Ok(n) => n,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "o servidor não respondeu a tempo",
                    ));
                }
                Err(error) => return Err(error),
            };
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "o servidor encerrou a conexão",
                ));
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }

    /// Lê uma resposta inteira, juntando as linhas de uma resposta de várias (`123-…` até
    /// `123 …`).
    pub(crate) fn read_reply(&mut self) -> io::Result<Reply> {
        if let Some(reply) = self.stashed.pop_front() {
            return Ok(reply);
        }
        let first = self.read_line()?;
        let first = self.charset.decode(&first);
        let code = first
            .get(..3)
            .and_then(|digits| digits.parse::<u16>().ok())
            .filter(|_| {
                first
                    .as_bytes()
                    .get(3)
                    .is_none_or(|&b| b == b' ' || b == b'-')
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "o servidor respondeu fora do protocolo FTP",
                )
            })?;
        let multi = first.as_bytes().get(3) == Some(&b'-');
        let mut lines = vec![first];
        if multi {
            let closing = format!("{code} ");
            loop {
                let line = self.read_line()?;
                let line = self.charset.decode(&line);
                let done = line.starts_with(&closing) || line == code.to_string();
                lines.push(line);
                if done {
                    break;
                }
            }
        }
        Ok(Reply { code, lines })
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn resposta(code: u16, linhas: &[&str]) -> Reply {
        Reply {
            code,
            lines: linhas.iter().map(|l| (*l).to_owned()).collect(),
        }
    }

    #[test]
    fn texto_pega_a_ultima_linha() {
        let r = resposta(211, &["211-Features:", " MLST", "211 End"]);
        assert_eq!(r.text(), "End");
        assert_eq!(r.body(), [" MLST".to_owned()]);
    }

    #[test]
    fn erros_ganham_o_tipo_certo() {
        let nao = reply_error(
            &resposta(550, &["550 No such file or directory."]),
            "ler",
            "/x",
        );
        assert_eq!(nao.kind(), io::ErrorKind::NotFound);
        assert!(nao.to_string().contains("/x"));
        let neg = reply_error(&resposta(550, &["550 Permission denied."]), "ler", "/x");
        assert_eq!(neg.kind(), io::ErrorKind::PermissionDenied);
        let caiu = reply_error(&resposta(421, &["421 Timeout."]), "ler", "/x");
        assert_eq!(caiu.kind(), io::ErrorKind::ConnectionReset);
        assert_eq!(reply_code(&caiu), Some(421));
        let nao_sei = reply_error(&resposta(502, &["502 Nope."]), "", "");
        assert_eq!(nao_sei.kind(), io::ErrorKind::Unsupported);
    }
}
