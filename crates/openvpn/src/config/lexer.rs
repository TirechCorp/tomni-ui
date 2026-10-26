//! O analisador léxico do `.ovpn`: linhas, aspas, escapes, comentários e blocos `<tag>…</tag>`.

use std::fmt;
use std::ops::Range;

/// Um item do arquivo: uma opção de uma linha ou um bloco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Item {
    /// `nome arg1 arg2`.
    Directive {
        /// O nome, sem o `--` do começo.
        name: String,
        /// Os argumentos, sem aspas.
        args: Vec<String>,
        /// A linha (de 1 em diante).
        line: usize,
        /// Os bytes da linha no texto original, com o fim de linha.
        span: Range<usize>,
    },
    /// `<tag>` … `</tag>`.
    Block {
        /// O nome da tag.
        tag: String,
        /// O conteúdo, linha a linha como veio.
        body: String,
        /// A linha da abertura.
        line: usize,
        /// Os bytes do bloco inteiro, das tags e do fim de linha.
        span: Range<usize>,
    },
}

impl Item {
    /// A linha de início.
    pub(crate) fn line(&self) -> usize {
        match self {
            Item::Directive { line, .. } | Item::Block { line, .. } => *line,
        }
    }

    /// Os bytes que o item ocupa no texto original.
    pub(crate) fn span(&self) -> Range<usize> {
        match self {
            Item::Directive { span, .. } | Item::Block { span, .. } => span.clone(),
        }
    }
}

/// O erro de sintaxe do arquivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LexError {
    /// Aspas abertas e não fechadas.
    UnclosedQuote { line: usize },
    /// Bloco aberto e nunca fechado.
    UnclosedBlock { tag: String, line: usize },
}

impl LexError {
    pub(crate) fn line(&self) -> usize {
        match self {
            LexError::UnclosedQuote { line } | LexError::UnclosedBlock { line, .. } => *line,
        }
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LexError::UnclosedQuote { .. } => f.write_str("aspas abertas e não fechadas"),
            LexError::UnclosedBlock { tag, .. } => {
                write!(f, "o bloco <{tag}> não foi fechado com </{tag}>")
            }
        }
    }
}

/// Quebra o texto em itens. `first_line` é o número da primeira linha (para os blocos
/// `<connection>`, que são analisados de novo por dentro).
pub(crate) fn lex(text: &str, first_line: usize) -> Result<Vec<Item>, LexError> {
    let mut items = Vec::new();
    let mut offset = 0usize;
    let mut line_no = first_line;
    let mut lines = text.split_inclusive('\n').peekable();

    while let Some(raw) = lines.next() {
        let start = offset;
        offset += raw.len();
        let this_line = line_no;
        line_no += 1;
        let content = raw.trim_end_matches(['\n', '\r']);
        let trimmed = content.trim();

        if let Some(tag) = open_tag(trimmed) {
            let close = format!("</{tag}>");
            let mut body = String::new();
            let mut closed = false;
            for inner in lines.by_ref() {
                offset += inner.len();
                line_no += 1;
                if inner.trim() == close {
                    closed = true;
                    break;
                }
                body.push_str(inner);
            }
            if !closed {
                return Err(LexError::UnclosedBlock {
                    tag: tag.to_string(),
                    line: this_line,
                });
            }
            // O corpo guarda o fim de linha de cada linha; o da última pode faltar.
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            items.push(Item::Block {
                tag: tag.to_string(),
                body,
                line: this_line,
                span: start..offset,
            });
            continue;
        }

        let mut tokens = tokenize(content, this_line)?;
        if tokens.is_empty() {
            continue;
        }
        let first = tokens.remove(0);
        let name = first
            .strip_prefix("--")
            .map(str::to_string)
            .unwrap_or(first);
        items.push(Item::Directive {
            name,
            args: tokens,
            line: this_line,
            span: start..offset,
        });
    }
    Ok(items)
}

/// Se a linha é só `<tag>`, devolve a tag.
fn open_tag(trimmed: &str) -> Option<&str> {
    let inner = trimmed.strip_prefix('<')?.strip_suffix('>')?;
    if inner.is_empty() || inner.starts_with('/') {
        return None;
    }
    if inner
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Some(inner)
    } else {
        None
    }
}

/// Separa a linha em argumentos: espaços separam; `"…"` aceita `\` para escapar; `'…'` é
/// literal; fora das aspas `\` escapa o próximo caractere; um argumento que começa por `#` ou
/// `;` (fora das aspas) é comentário até o fim da linha.
fn tokenize(line: &str, line_no: usize) -> Result<Vec<String>, LexError> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let Some(&first) = chars.peek() else { break };
        if first == '#' || first == ';' {
            break;
        }
        let mut token = String::new();
        let mut quote: Option<char> = None;
        loop {
            let Some(&c) = chars.peek() else {
                if quote.is_some() {
                    return Err(LexError::UnclosedQuote { line: line_no });
                }
                break;
            };
            match quote {
                None => {
                    if c.is_whitespace() {
                        break;
                    }
                    chars.next();
                    match c {
                        '"' | '\'' => quote = Some(c),
                        '\\' => {
                            if let Some(next) = chars.next() {
                                token.push(next);
                            }
                        }
                        _ => token.push(c),
                    }
                }
                Some('"') => {
                    chars.next();
                    match c {
                        '"' => quote = None,
                        '\\' => match chars.next() {
                            Some(next) => token.push(next),
                            None => return Err(LexError::UnclosedQuote { line: line_no }),
                        },
                        _ => token.push(c),
                    }
                }
                Some(_) => {
                    chars.next();
                    if c == '\'' {
                        quote = None;
                    } else {
                        token.push(c);
                    }
                }
            }
        }
        tokens.push(token);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(item: &Item) -> (&str, Vec<&str>) {
        match item {
            Item::Directive { name, args, .. } => {
                (name.as_str(), args.iter().map(String::as_str).collect())
            }
            Item::Block { tag, .. } => (tag.as_str(), vec![]),
        }
    }

    #[test]
    fn opcoes_simples_comentarios_e_traco_duplo() {
        let items = lex(
            "client\n# comentário\n; outro\n--remote host 1194 udp  # fim\n\n",
            1,
        )
        .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(dir(&items[0]), ("client", vec![]));
        assert_eq!(dir(&items[1]), ("remote", vec!["host", "1194", "udp"]));
        assert_eq!(items[1].line(), 4);
    }

    #[test]
    fn aspas_e_escapes() {
        let items = lex(
            "a \"com espaço\" 'sem \\escape' fora\\ do\\ lugar \"aspas \\\" dentro\"\n",
            1,
        )
        .unwrap();
        assert_eq!(
            dir(&items[0]),
            (
                "a",
                vec![
                    "com espaço",
                    "sem \\escape",
                    "fora do lugar",
                    "aspas \" dentro"
                ]
            )
        );
    }

    #[test]
    fn aspas_coladas_formam_um_argumento() {
        let items = lex("x ab\"cd ef\"gh\n", 1).unwrap();
        assert_eq!(dir(&items[0]), ("x", vec!["abcd efgh"]));
    }

    #[test]
    fn aspas_sem_fechar_dao_erro_com_a_linha() {
        let erro = lex("a\nb \"abre\n", 1).unwrap_err();
        assert_eq!(erro, LexError::UnclosedQuote { line: 2 });
    }

    #[test]
    fn blocos_guardam_o_conteudo_e_a_faixa() {
        let texto = "x 1\n<ca>\n-----BEGIN CERTIFICATE-----\nAAA\n</ca>\ny 2\n";
        let items = lex(texto, 1).unwrap();
        assert_eq!(items.len(), 3);
        let Item::Block {
            tag,
            body,
            line,
            span,
        } = &items[1]
        else {
            panic!("não é bloco")
        };
        assert_eq!(tag, "ca");
        assert_eq!(body, "-----BEGIN CERTIFICATE-----\nAAA\n");
        assert_eq!(*line, 2);
        assert_eq!(
            &texto[span.clone()],
            "<ca>\n-----BEGIN CERTIFICATE-----\nAAA\n</ca>\n"
        );
        assert_eq!(items[2].line(), 6);
    }

    #[test]
    fn bloco_sem_fechar_da_erro() {
        let erro = lex("<key>\nabc\n", 1).unwrap_err();
        assert_eq!(
            erro,
            LexError::UnclosedBlock {
                tag: "key".into(),
                line: 1
            }
        );
        assert!(erro.to_string().contains("</key>"));
    }

    #[test]
    fn fim_de_linha_do_windows() {
        let items = lex("remote host 1194\r\n<ca>\r\nabc\r\n</ca>\r\n", 1).unwrap();
        assert_eq!(dir(&items[0]), ("remote", vec!["host", "1194"]));
        let Item::Block { body, .. } = &items[1] else {
            panic!()
        };
        assert_eq!(body, "abc\r\n");
    }

    #[test]
    fn a_numeracao_pode_comecar_depois() {
        let items = lex("remote a\n", 10).unwrap();
        assert_eq!(items[0].line(), 10);
    }
}
