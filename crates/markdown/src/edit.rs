//! As edições do Markdown, como funções puras sobre o texto: cada uma recebe o texto e a
//! seleção (em bytes) e devolve o [`Change`] que o app aplica pelo controlador do editor, ou
//! `None` quando a edição não se aplica (o app segue o comportamento padrão da tecla).
//!
//! Todas as posições são em bytes de `text` e caem em limites de caractere; uma posição fora do
//! texto ou no meio de um caractere devolve `None`.

use std::ops::Range;

/// Uma troca no texto: no lugar de `range` entra `insert`, e depois dela a seleção é `selection`
/// (posições do texto **novo**).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// O trecho do texto antigo que sai.
    pub range: Range<usize>,
    /// O que entra no lugar.
    pub insert: String,
    /// A seleção depois da troca, no texto novo. Vazia é só o cursor.
    pub selection: Range<usize>,
}

impl Change {
    /// O texto depois da troca. Serve nos testes e a quem edita um `String` sem controlador.
    ///
    /// ```
    /// use tomni_ui_markdown::edit::toggle_wrap;
    ///
    /// let texto = "um dois";
    /// let troca = toggle_wrap(texto, 3..7, "**").unwrap();
    /// assert_eq!(troca.apply(texto), "um **dois**");
    /// ```
    pub fn apply(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + self.insert.len());
        out.push_str(&text[..self.range.start]);
        out.push_str(&self.insert);
        out.push_str(&text[self.range.end..]);
        out
    }
}

/// O prefixo de linha de [`toggle_prefix`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefix {
    /// `- `
    Bullet,
    /// `1. `, `2. `…
    Numbered,
    /// `- [ ] `
    Task,
    /// `> `
    Quote,
    /// `# ` a `###### `; o nível vai de 1 a 6.
    Heading(u8),
}

/// O bloco de [`insert_block`] e de [`surround`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block<'a> {
    /// Um bloco de código cercado, com o nome da linguagem (pode ser vazio).
    Code(&'a str),
    /// Uma tabela com `cols` colunas e `rows` linhas além do cabeçalho.
    Table {
        /// O número de colunas (no mínimo 1).
        cols: usize,
        /// O número de linhas do corpo.
        rows: usize,
    },
    /// Uma linha horizontal (`---`).
    Rule,
    /// Um link, `[texto](endereço)`.
    Link,
}

fn valid(text: &str, range: &Range<usize>) -> bool {
    range.start <= range.end
        && range.end <= text.len()
        && text.is_char_boundary(range.start)
        && text.is_char_boundary(range.end)
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, pos: usize) -> usize {
    text[pos..].find('\n').map_or(text.len(), |i| pos + i)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A palavra em volta de `pos` (vazia, em `pos..pos`, se não houver).
fn word_at(text: &str, pos: usize) -> Range<usize> {
    let start = text[..pos]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(pos, |(i, _)| i);
    let end = text[pos..]
        .char_indices()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(pos, |(i, c)| pos + i + c.len_utf8());
    start..end
}

/// Envolve a seleção em `mark` (`**`, `*`, `~~`, `` ` ``…), ou tira o `mark` se ela já estiver
/// envolvida. Sem seleção, vale a palavra do cursor; sem palavra, entra o par de marcas com o
/// cursor no meio. Os espaços das pontas da seleção ficam de fora das marcas.
///
/// ```
/// use tomni_ui_markdown::edit::toggle_wrap;
///
/// let texto = "um dois";
/// let ligado = toggle_wrap(texto, 3..7, "**").unwrap().apply(texto);
/// assert_eq!(ligado, "um **dois**");
/// let desligado = toggle_wrap(&ligado, 5..9, "**").unwrap().apply(&ligado);
/// assert_eq!(desligado, texto);
/// ```
pub fn toggle_wrap(text: &str, sel: Range<usize>, mark: &str) -> Option<Change> {
    if mark.is_empty() || !valid(text, &sel) {
        return None;
    }
    let target = if sel.is_empty() {
        word_at(text, sel.start)
    } else {
        let selected = &text[sel.clone()];
        let head = selected.len() - selected.trim_start().len();
        let tail = selected.len() - selected.trim_end().len();
        if head + tail >= selected.len() {
            sel.start..sel.start
        } else {
            sel.start + head..sel.end - tail
        }
    };
    let ml = mark.len();
    // Já envolvida por fora da seleção: `**[dois]**`. Com uma marca de um caractere só (`*`),
    // as marcas coladas contam em pares: `**` é negrito, e só uma sobra (`***`) é itálico.
    let before = &text[..target.start];
    let after = &text[target.end..];
    let mut chars = mark.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    let odd_run = single.is_none_or(|c| {
        let behind = before.chars().rev().take_while(|&x| x == c).count();
        let ahead = after.chars().take_while(|&x| x == c).count();
        behind.min(ahead) % 2 == 1
    });
    if before.ends_with(mark) && after.starts_with(mark) && odd_run {
        let range = target.start - ml..target.end + ml;
        let inner = &text[target.clone()];
        let start = range.start;
        return Some(Change {
            range,
            insert: inner.to_owned(),
            selection: start..start + inner.len(),
        });
    }
    // Já envolvida por dentro da seleção: `[**dois**]`.
    let inner = &text[target.clone()];
    if inner.len() >= 2 * ml && inner.starts_with(mark) && inner.ends_with(mark) {
        let plain = &inner[ml..inner.len() - ml];
        return Some(Change {
            range: target.clone(),
            insert: plain.to_owned(),
            selection: target.start..target.start + plain.len(),
        });
    }
    let selected_len = inner.len();
    Some(Change {
        range: target.clone(),
        insert: format!("{mark}{inner}{mark}"),
        selection: target.start + ml..target.start + ml + selected_len,
    })
}

/// O que `marker` achou no começo de uma linha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Heading(u8),
    Quote,
    Task,
    Bullet,
    Numbered,
}

#[derive(Clone, Copy, Debug)]
struct Marker {
    kind: Kind,
    /// Os bytes de recuo, antes do marcador.
    indent: usize,
    /// Onde o conteúdo começa (depois do marcador e do espaço).
    end: usize,
    bullet: u8,
    number: u64,
    delim: u8,
}

/// O marcador de bloco no começo de `line`: título, citação, tarefa, item ou item numerado.
fn marker(line: &str) -> Option<Marker> {
    let bytes = line.as_bytes();
    let indent = bytes
        .iter()
        .take_while(|&&c| c == b' ' || c == b'\t')
        .count();
    let rest = &bytes[indent..];
    let mut m = Marker {
        kind: Kind::Bullet,
        indent,
        end: indent,
        bullet: b'-',
        number: 0,
        delim: b'.',
    };
    match *rest.first()? {
        b'#' => {
            let n = rest.iter().take_while(|&&c| c == b'#').count();
            if n > 6 {
                return None;
            }
            m.end = match rest.get(n) {
                None => indent + n,
                Some(b' ') => indent + n + 1,
                Some(_) => return None,
            };
            m.kind = Kind::Heading(n as u8);
        }
        b'>' => {
            m.end = indent + 1 + usize::from(rest.get(1) == Some(&b' '));
            m.kind = Kind::Quote;
        }
        c @ (b'-' | b'*' | b'+') => {
            if rest.get(1) != Some(&b' ') {
                return None;
            }
            m.bullet = c;
            m.end = indent + 2;
            if rest.len() >= 5
                && matches!(&rest[2..5], b"[ ]" | b"[x]" | b"[X]")
                && matches!(rest.get(5), None | Some(b' '))
            {
                m.kind = Kind::Task;
                m.end = indent + 5 + usize::from(rest.get(5).is_some());
            }
        }
        b'0'..=b'9' => {
            let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
            if digits > 9 {
                return None;
            }
            let delim = *rest.get(digits)?;
            if !matches!(delim, b'.' | b')') || rest.get(digits + 1) != Some(&b' ') {
                return None;
            }
            m.number = line[indent..indent + digits].parse().ok()?;
            m.delim = delim;
            m.end = indent + digits + 2;
            m.kind = Kind::Numbered;
        }
        _ => return None,
    }
    Some(m)
}

/// As linhas tocadas pela seleção: de onde a primeira começa até onde a última acaba. Uma
/// seleção que termina no começo de uma linha não toca nela.
fn touched(text: &str, sel: &Range<usize>) -> Range<usize> {
    let start = line_start(text, sel.start);
    let last = if sel.end > sel.start && text[..sel.end].ends_with('\n') {
        sel.end - 1
    } else {
        sel.end
    };
    start..line_end(text, last)
}

/// Põe o prefixo em todas as linhas tocadas pela seleção, ou tira se todas já o têm. Trocar um
/// prefixo por outro (título 1 por título 2, lista por tarefa) troca o marcador; numerar
/// renumera a partir de 1. As linhas em branco ficam como estão, a menos que sejam as únicas.
///
/// ```
/// use tomni_ui_markdown::edit::{toggle_prefix, Prefix};
///
/// let texto = "a\nb\nc";
/// let lista = toggle_prefix(texto, 0..texto.len(), Prefix::Bullet).unwrap().apply(texto);
/// assert_eq!(lista, "- a\n- b\n- c");
/// let numerada = toggle_prefix(&lista, 0..lista.len(), Prefix::Numbered).unwrap().apply(&lista);
/// assert_eq!(numerada, "1. a\n2. b\n3. c");
/// ```
pub fn toggle_prefix(text: &str, sel: Range<usize>, prefix: Prefix) -> Option<Change> {
    if !valid(text, &sel) {
        return None;
    }
    if let Prefix::Heading(level) = prefix
        && !(1..=6).contains(&level)
    {
        return None;
    }
    let span = touched(text, &sel);
    let lines: Vec<&str> = text[span.clone()].split('\n').collect();
    let blank = |line: &str| line.trim().is_empty();
    let all_blank = lines.iter().all(|l| blank(l));
    let applies: Vec<bool> = lines.iter().map(|l| all_blank || !blank(l)).collect();
    let same = |m: &Marker| match (prefix, m.kind) {
        (Prefix::Bullet, Kind::Bullet)
        | (Prefix::Numbered, Kind::Numbered)
        | (Prefix::Task, Kind::Task)
        | (Prefix::Quote, Kind::Quote) => true,
        (Prefix::Heading(a), Kind::Heading(b)) => a == b,
        _ => false,
    };
    let remove = lines
        .iter()
        .zip(&applies)
        .filter(|&(_, &applies)| applies)
        .all(|(line, _)| marker(line).is_some_and(|m| same(&m)));

    let mut out = String::new();
    let mut number = 1;
    let mut caret = None;
    let mut line_abs = span.start;
    for (line, &applies) in lines.iter().zip(&applies) {
        if !out.is_empty() || line_abs > span.start {
            out.push('\n');
        }
        let new_start = out.len();
        if !applies {
            out.push_str(line);
            if sel.is_empty() && (line_abs..=line_abs + line.len()).contains(&sel.start) {
                caret = Some(new_start + sel.start - line_abs);
            }
            line_abs += line.len() + 1;
            continue;
        }
        let (indent, old_head) = match marker(line) {
            Some(m) => (m.indent, m.end),
            None => {
                let ws = line.len() - line.trim_start().len();
                (ws, ws)
            }
        };
        let kept = if matches!(prefix, Prefix::Heading(_)) && !remove {
            ""
        } else {
            &line[..indent]
        };
        let added = if remove {
            String::new()
        } else {
            match prefix {
                Prefix::Bullet => "- ".to_owned(),
                Prefix::Task => "- [ ] ".to_owned(),
                Prefix::Quote => "> ".to_owned(),
                Prefix::Heading(level) => format!("{} ", "#".repeat(usize::from(level))),
                Prefix::Numbered => {
                    let n = number;
                    number += 1;
                    format!("{n}. ")
                }
            }
        };
        out.push_str(kept);
        out.push_str(&added);
        let new_head = kept.len() + added.len();
        out.push_str(&line[old_head..]);
        if sel.is_empty() && (line_abs..=line_abs + line.len()).contains(&sel.start) {
            let offset = sel.start - line_abs;
            caret = Some(new_start + new_head + offset.saturating_sub(old_head));
        }
        line_abs += line.len() + 1;
    }
    let selection = match caret {
        Some(c) => span.start + c..span.start + c,
        None => span.start..span.start + out.len(),
    };
    Some(Change {
        range: span,
        insert: out,
        selection,
    })
}

/// O que o Enter faz numa linha de lista, de tarefa ou de citação: insere a próxima, com o mesmo
/// marcador (o número mais um, a tarefa desmarcada). Num item vazio, tira o marcador e termina a
/// lista. Em qualquer outra linha, ou com o cursor dentro do marcador, devolve `None` e o Enter
/// segue o padrão.
///
/// ```
/// use tomni_ui_markdown::edit::continue_list;
///
/// let texto = "1. um";
/// let troca = continue_list(texto, texto.len()).unwrap();
/// assert_eq!(troca.apply(texto), "1. um\n2. ");
/// ```
pub fn continue_list(text: &str, caret: usize) -> Option<Change> {
    if !valid(text, &(caret..caret)) {
        return None;
    }
    let start = line_start(text, caret);
    let end = line_end(text, caret);
    let line = &text[start..end];
    let m = marker(line)?;
    if matches!(m.kind, Kind::Heading(_)) || caret - start < m.end {
        return None;
    }
    if line[m.end..].trim().is_empty() {
        return Some(Change {
            range: start..end,
            insert: String::new(),
            selection: start..start,
        });
    }
    let next = match m.kind {
        Kind::Bullet => format!("{} ", m.bullet as char),
        Kind::Task => format!("{} [ ] ", m.bullet as char),
        Kind::Numbered => format!("{}{} ", m.number + 1, m.delim as char),
        Kind::Quote => "> ".to_owned(),
        Kind::Heading(_) => return None,
    };
    let insert = format!("\n{}{}", &line[..m.indent], next);
    let at = caret + insert.len();
    Some(Change {
        range: caret..caret,
        insert,
        selection: at..at,
    })
}

/// O Tab (`out == false`) e o Shift+Tab (`out == true`) nas linhas de lista tocadas pela
/// seleção: recua ou desfaz o recuo de 2 espaços (nos itens numerados, a largura do número, para
/// o item continuar dentro do de cima). Sem nenhuma linha de lista, ou sem recuo para desfazer,
/// devolve `None` e a tecla segue o padrão.
///
/// ```
/// use tomni_ui_markdown::edit::indent;
///
/// let texto = "- a\n- b";
/// let troca = indent(texto, 4..4, false).unwrap();
/// assert_eq!(troca.apply(texto), "- a\n  - b");
/// ```
pub fn indent(text: &str, sel: Range<usize>, out: bool) -> Option<Change> {
    if !valid(text, &sel) {
        return None;
    }
    let span = touched(text, &sel);
    let mut result = String::new();
    let mut changed = false;
    let mut caret = None;
    let mut line_abs = span.start;
    for (i, line) in text[span.clone()].split('\n').enumerate() {
        if i > 0 {
            result.push('\n');
        }
        let new_start = result.len();
        let list =
            marker(line).filter(|m| matches!(m.kind, Kind::Bullet | Kind::Task | Kind::Numbered));
        let mut moved: isize = 0;
        match list {
            Some(m) => {
                let step = if m.kind == Kind::Numbered {
                    m.end - m.indent
                } else {
                    2
                };
                if out {
                    let leading = &line[..m.indent];
                    let remove = if leading.starts_with('\t') {
                        1
                    } else {
                        leading.bytes().take_while(|&c| c == b' ').count().min(step)
                    };
                    result.push_str(&line[remove..]);
                    moved = -(remove as isize);
                    changed |= remove > 0;
                } else {
                    result.push_str(&" ".repeat(step));
                    result.push_str(line);
                    moved = step as isize;
                    changed = true;
                }
            }
            None => result.push_str(line),
        }
        if sel.is_empty() && (line_abs..=line_abs + line.len()).contains(&sel.start) {
            let offset = (sel.start - line_abs) as isize;
            caret = Some(new_start + (offset + moved).max(0) as usize);
        }
        line_abs += line.len() + 1;
    }
    if !changed {
        return None;
    }
    let selection = match caret {
        Some(c) => span.start + c..span.start + c,
        None => span.start..span.start + result.len(),
    };
    Some(Change {
        range: span,
        insert: result,
        selection,
    })
}

/// O texto do bloco (sem as linhas em branco em volta) e a seleção dentro dele.
fn block_text(block: Block<'_>) -> (String, Range<usize>) {
    match block {
        Block::Code(lang) => {
            let at = if lang.is_empty() {
                3
            } else {
                3 + lang.len() + 1
            };
            (format!("```{lang}\n\n```"), at..at)
        }
        Block::Table { cols, rows } => {
            let cols = cols.max(1);
            let mut head = String::from("|");
            for n in 1..=cols {
                head.push_str(&format!(" Coluna {n} |"));
            }
            let first = 2..2 + "Coluna 1".len();
            let divider = format!("|{}", " --- |".repeat(cols));
            let row = format!("|{}", "   |".repeat(cols));
            let mut table = format!("{head}\n{divider}");
            for _ in 0..rows {
                table.push('\n');
                table.push_str(&row);
            }
            (table, first)
        }
        Block::Rule => ("---".to_owned(), 3..3),
        Block::Link => ("[texto](endereço)".to_owned(), 1..1 + "texto".len()),
    }
}

/// Troca `range` pelo bloco, em linha própria: com uma linha em branco antes e depois quando o
/// texto em volta não tem uma. `rel` é a seleção dentro do bloco.
fn place(text: &str, range: Range<usize>, block: &str, rel: Range<usize>) -> Change {
    let head = &text[..range.start];
    let tail = &text[range.end..];
    let pre = if head.trim().is_empty() || head.ends_with("\n\n") {
        ""
    } else if head.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let post = if tail.is_empty() || tail.starts_with("\n\n") {
        ""
    } else if tail.starts_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let shift = range.start + pre.len();
    Change {
        range,
        insert: format!("{pre}{block}{post}"),
        selection: shift + rel.start..shift + rel.end,
    }
}

/// Insere o bloco no cursor. Código, tabela e linha horizontal ficam em linha própria, com uma
/// linha em branco antes e depois quando preciso (no meio de um parágrafo, o texto de depois do
/// cursor desce para depois do bloco); o link entra no lugar, no meio do texto. A seleção fica
/// onde escrever: o nome da linguagem (ou o corpo, se a linguagem foi dada), a primeira célula,
/// o texto do link.
///
/// ```
/// use tomni_ui_markdown::edit::{insert_block, Block};
///
/// let texto = "abc";
/// let troca = insert_block(texto, 3, Block::Rule).unwrap();
/// assert_eq!(troca.apply(texto), "abc\n\n---");
/// ```
pub fn insert_block(text: &str, caret: usize, block: Block<'_>) -> Option<Change> {
    if !valid(text, &(caret..caret)) {
        return None;
    }
    let (body, rel) = block_text(block);
    Some(if block == Block::Link {
        Change {
            range: caret..caret,
            insert: body,
            selection: caret + rel.start..caret + rel.end,
        }
    } else {
        place(text, caret..caret, &body, rel)
    })
}

/// Como [`insert_block`], mas levando a seleção para dentro do bloco: um bloco de código cerca o
/// texto selecionado (com a seleção no corpo), e um link vira `[seleção](` com o cursor no
/// endereço. Sem seleção, é o [`insert_block`]. Tabela e linha horizontal ignoram a seleção e
/// entram depois dela.
///
/// ```
/// use tomni_ui_markdown::edit::{surround, Block};
///
/// let texto = "veja o site";
/// let troca = surround(texto, 7..11, Block::Link).unwrap();
/// assert_eq!(troca.apply(texto), "veja o [site]()");
/// assert_eq!(troca.selection, 14..14);
/// ```
pub fn surround(text: &str, sel: Range<usize>, block: Block<'_>) -> Option<Change> {
    if !valid(text, &sel) {
        return None;
    }
    if sel.is_empty() {
        return insert_block(text, sel.start, block);
    }
    match block {
        Block::Code(lang) => {
            let selected = text[sel.clone()].trim_end_matches(['\n', '\r']);
            let range = sel.start..sel.start + selected.len();
            let open = format!("```{lang}\n");
            let body = format!("{open}{selected}\n```");
            Some(place(
                text,
                range,
                &body,
                open.len()..open.len() + selected.len(),
            ))
        }
        Block::Link => {
            let selected = &text[sel.clone()];
            let at = sel.start + selected.len() + 3;
            Some(Change {
                range: sel,
                insert: format!("[{selected}]()"),
                selection: at..at,
            })
        }
        Block::Table { .. } | Block::Rule => insert_block(text, sel.end, block),
    }
}

/// Troca `[ ]` por `[x]` (e `[x]` por `[ ]`) no marcador de tarefa que começa em `offset`, o
/// byte do `[` que o `on_task` do `MarkdownView` entrega. A seleção devolvida é só o cursor logo
/// depois do marcador; quem aplica pode manter a seleção que o editor já tem.
///
/// ```
/// use tomni_ui_markdown::edit::toggle_task;
///
/// let texto = "- [ ] fazer";
/// let troca = toggle_task(texto, 2).unwrap();
/// assert_eq!(troca.apply(texto), "- [x] fazer");
/// ```
pub fn toggle_task(text: &str, offset: usize) -> Option<Change> {
    let marker = text.get(offset..offset + 3)?;
    let insert = match marker {
        "[ ]" => "x",
        "[x]" | "[X]" => " ",
        _ => return None,
    };
    Some(Change {
        range: offset + 1..offset + 2,
        insert: insert.to_owned(),
        selection: offset + 3..offset + 3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aplica(text: &str, change: Option<Change>) -> (String, Range<usize>) {
        let change = change.expect("a edição devia se aplicar");
        (change.apply(text), change.selection)
    }

    #[test]
    fn toggle_wrap_liga_e_desliga_em_ida_e_volta() {
        let texto = "um dois três";
        for mark in ["**", "*", "~~", "`"] {
            let (ligado, sel) = aplica(texto, toggle_wrap(texto, 3..7, mark));
            assert_eq!(ligado, format!("um {mark}dois{mark} três"));
            assert_eq!(&ligado[sel.clone()], "dois");
            let (volta, sel) = aplica(&ligado, toggle_wrap(&ligado, sel, mark));
            assert_eq!(volta, texto, "{mark}");
            assert_eq!(&volta[sel], "dois");
        }
    }

    #[test]
    fn toggle_wrap_pega_a_palavra_do_cursor_e_tira_pelas_marcas_dentro() {
        let texto = "olá mundo";
        let (ligado, _) = aplica(texto, toggle_wrap(texto, 6..6, "**"));
        assert_eq!(ligado, "olá **mundo**");
        // Seleção que já inclui as marcas.
        let (volta, _) = aplica(&ligado, toggle_wrap(&ligado, 4..ligado.len(), "**"));
        assert_eq!(volta, texto);
        // Cursor logo depois da palavra, sem palavra do outro lado: vale a palavra de antes.
        let (ligado, _) = aplica(texto, toggle_wrap(texto, 4..4, "*"));
        assert_eq!(ligado, "*olá* mundo");
    }

    #[test]
    fn toggle_wrap_sem_palavra_abre_o_par_e_deixa_os_espacos_de_fora() {
        let texto = "a  b";
        let (par, sel) = aplica(texto, toggle_wrap(texto, 2..2, "**"));
        assert_eq!(par, "a **** b");
        assert_eq!(sel, 4..4);
        // O par vazio some no segundo toque.
        let (volta, _) = aplica(&par, toggle_wrap(&par, 4..4, "**"));
        assert_eq!(volta, texto);
        let texto = "um  dois  fim";
        let (ligado, _) = aplica(texto, toggle_wrap(texto, 2..10, "**"));
        assert_eq!(ligado, "um  **dois**  fim");
    }

    #[test]
    fn toggle_wrap_itálico_dentro_de_negrito_nao_confunde() {
        let texto = "**dois**";
        let (ligado, _) = aplica(texto, toggle_wrap(texto, 2..6, "*"));
        assert_eq!(ligado, "***dois***");
        let (volta, _) = aplica(&ligado, toggle_wrap(&ligado, 3..7, "*"));
        assert_eq!(volta, texto);
    }

    #[test]
    fn toggle_wrap_com_acento_e_emoji() {
        let texto = "ação 🎉 fim";
        let inicio = texto.find('🎉').unwrap();
        let (ligado, _) = aplica(texto, toggle_wrap(texto, inicio..inicio + 4, "~~"));
        assert_eq!(ligado, "ação ~~🎉~~ fim");
        let (ligado, sel) = aplica(texto, toggle_wrap(texto, 1..1, "**"));
        assert_eq!(ligado, "**ação** 🎉 fim");
        assert_eq!(&ligado[sel], "ação");
        // No meio de um caractere não se aplica.
        assert!(toggle_wrap(texto, 1..2, "**").is_none());
        assert!(toggle_wrap(texto, 0..99, "**").is_none());
    }

    #[test]
    fn toggle_prefix_em_tres_linhas_e_de_volta() {
        let texto = "a\nb\nc";
        let todo = 0..texto.len();
        let (lista, sel) = aplica(texto, toggle_prefix(texto, todo, Prefix::Bullet));
        assert_eq!(lista, "- a\n- b\n- c");
        assert_eq!(sel, 0..lista.len());
        let (numerada, _) = aplica(
            &lista,
            toggle_prefix(&lista, 0..lista.len(), Prefix::Numbered),
        );
        assert_eq!(numerada, "1. a\n2. b\n3. c");
        let (tarefas, _) = aplica(
            &numerada,
            toggle_prefix(&numerada, 0..numerada.len(), Prefix::Task),
        );
        assert_eq!(tarefas, "- [ ] a\n- [ ] b\n- [ ] c");
        let (citacao, _) = aplica(
            &tarefas,
            toggle_prefix(&tarefas, 0..tarefas.len(), Prefix::Quote),
        );
        assert_eq!(citacao, "> a\n> b\n> c");
        let (puro, _) = aplica(
            &citacao,
            toggle_prefix(&citacao, 0..citacao.len(), Prefix::Quote),
        );
        assert_eq!(puro, texto);
    }

    #[test]
    fn toggle_prefix_so_toca_as_linhas_da_selecao_e_pula_as_em_branco() {
        let texto = "a\n\nb\nc";
        // Seleção de "a" até o começo de "b": a linha de "b" não é tocada.
        let (lista, _) = aplica(texto, toggle_prefix(texto, 0..3, Prefix::Numbered));
        assert_eq!(lista, "1. a\n\nb\nc");
        let (lista, _) = aplica(
            texto,
            toggle_prefix(texto, 0..texto.len(), Prefix::Numbered),
        );
        assert_eq!(lista, "1. a\n\n2. b\n3. c");
        // Uma linha vazia sozinha recebe o prefixo.
        let (vazia, sel) = aplica("", toggle_prefix("", 0..0, Prefix::Bullet));
        assert_eq!(vazia, "- ");
        assert_eq!(sel, 2..2);
    }

    #[test]
    fn toggle_prefix_troca_titulo_e_leva_o_cursor() {
        let texto = "# Título";
        let (dois, sel) = aplica(texto, toggle_prefix(texto, 3..3, Prefix::Heading(2)));
        assert_eq!(dois, "## Título");
        assert_eq!(sel, 4..4);
        let (sem, _) = aplica(&dois, toggle_prefix(&dois, 4..4, Prefix::Heading(2)));
        assert_eq!(sem, "Título");
        let (lista, _) = aplica("- item", toggle_prefix("- item", 0..0, Prefix::Heading(3)));
        assert_eq!(lista, "### item");
        assert!(toggle_prefix(texto, 0..0, Prefix::Heading(7)).is_none());
        assert!(toggle_prefix(texto, 0..0, Prefix::Heading(0)).is_none());
    }

    #[test]
    fn toggle_prefix_renumera_a_partir_de_um_e_mantem_o_recuo() {
        let texto = "3. a\n7. b\n  - c";
        let (novo, _) = aplica(
            texto,
            toggle_prefix(texto, 0..texto.len(), Prefix::Numbered),
        );
        assert_eq!(novo, "1. a\n2. b\n  3. c");
    }

    #[test]
    fn continue_list_em_lista_numerada_tarefa_e_citacao() {
        let (novo, sel) = aplica("- um", continue_list("- um", 4));
        assert_eq!(novo, "- um\n- ");
        assert_eq!(sel, 7..7);
        let (novo, _) = aplica("* um", continue_list("* um", 4));
        assert_eq!(novo, "* um\n* ");
        let (novo, _) = aplica("9. nove", continue_list("9. nove", 7));
        assert_eq!(novo, "9. nove\n10. ");
        let (novo, _) = aplica("- [x] feito", continue_list("- [x] feito", 11));
        assert_eq!(novo, "- [x] feito\n- [ ] ");
        let (novo, _) = aplica("> fala", continue_list("> fala", 6));
        assert_eq!(novo, "> fala\n> ");
        let (novo, _) = aplica("  - fundo", continue_list("  - fundo", 9));
        assert_eq!(novo, "  - fundo\n  - ");
    }

    #[test]
    fn continue_list_no_meio_do_item_e_em_item_vazio() {
        let (novo, sel) = aplica("- ab cd", continue_list("- ab cd", 4));
        assert_eq!(novo, "- ab\n-  cd");
        assert_eq!(sel, 7..7);
        // Item vazio: tira o marcador e termina a lista.
        let texto = "- a\n- ";
        let (novo, sel) = aplica(texto, continue_list(texto, texto.len()));
        assert_eq!(novo, "- a\n");
        assert_eq!(sel, 4..4);
        let texto = "1. ";
        assert_eq!(aplica(texto, continue_list(texto, 3)).0, "");
        let texto = "- [ ] ";
        assert_eq!(aplica(texto, continue_list(texto, 6)).0, "");
    }

    #[test]
    fn continue_list_nao_se_aplica_fora_de_lista() {
        assert!(continue_list("texto", 5).is_none());
        assert!(continue_list("# título", 8).is_none());
        // Cursor dentro do marcador.
        assert!(continue_list("- a", 1).is_none());
        assert!(continue_list("- a", 99).is_none());
        // Linha de régua não é item.
        assert!(continue_list("---", 3).is_none());
    }

    #[test]
    fn indent_recua_e_desfaz_so_em_lista() {
        let texto = "- a\n- b";
        let (dentro, sel) = aplica(texto, indent(texto, 7..7, false));
        assert_eq!(dentro, "- a\n  - b");
        assert_eq!(sel, 9..9);
        let (fora, sel) = aplica(&dentro, indent(&dentro, 9..9, true));
        assert_eq!(fora, texto);
        assert_eq!(sel, 7..7);
        // Duas linhas selecionadas.
        let (duas, sel) = aplica(texto, indent(texto, 0..texto.len(), false));
        assert_eq!(duas, "  - a\n  - b");
        assert_eq!(sel, 0..duas.len());
        // Item numerado recua pela largura do número, para continuar dentro do de cima.
        let numerada = "1. a\n2. b";
        let (dentro, _) = aplica(numerada, indent(numerada, 9..9, false));
        assert_eq!(dentro, "1. a\n   2. b");
        // Fora de lista, ou sem recuo para desfazer, não se aplica.
        assert!(indent("texto", 0..0, false).is_none());
        assert!(indent("# título", 0..0, false).is_none());
        assert!(indent("- a", 0..0, true).is_none());
        assert!(indent("- a", 0..9, false).is_none());
    }

    #[test]
    fn insert_block_em_linha_propria() {
        let (novo, sel) = aplica("", insert_block("", 0, Block::Code("")));
        assert_eq!(novo, "```\n\n```");
        assert_eq!(sel, 3..3);
        let texto = "abcdef";
        let (novo, sel) = aplica(texto, insert_block(texto, 3, Block::Code("sh")));
        assert_eq!(novo, "abc\n\n```sh\n\n```\n\ndef");
        // Com a linguagem dada, o cursor vai para o corpo.
        assert_eq!(sel, 11..11);
        assert_eq!(&novo[..sel.start], "abc\n\n```sh\n");
        let texto = "um\n\ndois";
        let (novo, _) = aplica(texto, insert_block(texto, 4, Block::Rule));
        assert_eq!(novo, "um\n\n---\n\ndois");
        let texto = "um\ndois";
        let (novo, _) = aplica(texto, insert_block(texto, 3, Block::Rule));
        assert_eq!(novo, "um\n\n---\n\ndois");
    }

    #[test]
    fn insert_block_tabela_e_link() {
        let (novo, sel) = aplica("", insert_block("", 0, Block::Table { cols: 2, rows: 2 }));
        assert_eq!(
            novo,
            "| Coluna 1 | Coluna 2 |\n| --- | --- |\n|   |   |\n|   |   |"
        );
        assert_eq!(&novo[sel], "Coluna 1");
        let texto = "veja ";
        let (novo, sel) = aplica(texto, insert_block(texto, 5, Block::Link));
        assert_eq!(novo, "veja [texto](endereço)");
        assert_eq!(&novo[sel], "texto");
        assert!(insert_block(texto, 99, Block::Rule).is_none());
        // A tabela gerada é uma tabela para o parser.
        let blocos = crate::parse::parse(
            &aplica("", insert_block("", 0, Block::Table { cols: 3, rows: 1 })).0,
        );
        assert!(matches!(
            blocos.as_slice(),
            [crate::parse::Block::Table { .. }]
        ));
    }

    #[test]
    fn surround_cerca_a_selecao() {
        let texto = "antes\nsegredo\ndepois";
        let ini = texto.find("segredo").unwrap();
        let (novo, sel) = aplica(texto, surround(texto, ini..ini + 7, Block::Code("segredo")));
        assert_eq!(novo, "antes\n\n```segredo\nsegredo\n```\n\ndepois");
        assert_eq!(&novo[sel], "segredo");
        let texto = "veja o site agora";
        let (novo, sel) = aplica(texto, surround(texto, 7..11, Block::Link));
        assert_eq!(novo, "veja o [site]() agora");
        assert_eq!(sel, 14..14);
        // Sem seleção é o insert_block.
        let (novo, _) = aplica("a", surround("a", 1..1, Block::Rule));
        assert_eq!(novo, "a\n\n---");
    }

    #[test]
    fn toggle_task_nos_dois_sentidos() {
        let texto = "- [ ] a\n- [x] b\n- [X] c";
        let (um, _) = aplica(texto, toggle_task(texto, 2));
        assert_eq!(um, "- [x] a\n- [x] b\n- [X] c");
        let (dois, _) = aplica(&um, toggle_task(&um, 10));
        assert_eq!(dois, "- [x] a\n- [ ] b\n- [X] c");
        let (tres, _) = aplica(&dois, toggle_task(&dois, 18));
        assert_eq!(tres, "- [x] a\n- [ ] b\n- [ ] c");
        assert!(toggle_task(texto, 0).is_none());
        assert!(toggle_task(texto, 999).is_none());
    }
}
