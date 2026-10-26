//! Os links e as imagens de um texto, com a faixa do destino, para o app reescrever.

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Parser, Tag};

use crate::parse::options;

/// Um link ou uma imagem do texto: o destino e onde ele está escrito.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRef {
    /// O destino, como está escrito (sem os `<>` de um destino entre `<…>`).
    pub dest: String,
    /// A faixa em bytes do destino no texto. Num link de referência (`[a][1]`), é a do destino
    /// na definição (`[1]: url`), e vários links da mesma definição repetem a faixa.
    pub dest_range: Range<usize>,
    /// Se é uma imagem (`![alt](url)`), e não um link.
    pub image: bool,
}

/// Cada link e cada imagem de `source`, na ordem em que aparecem. Código (inline ou em bloco)
/// não conta, porque não é link.
///
/// ```
/// use tomni_ui_markdown::links;
///
/// let texto = "Veja [o manual](manual.md) e ![foto](img/a.png).";
/// let achados = links(texto);
/// assert_eq!(achados.len(), 2);
/// assert_eq!(&texto[achados[0].dest_range.clone()], "manual.md");
/// assert!(achados[1].image);
/// ```
pub fn links(source: &str) -> Vec<LinkRef> {
    let mut iter = Parser::new_ext(source, options()).into_offset_iter();
    let events: Vec<(Event<'_>, Range<usize>)> = iter.by_ref().collect();
    let definitions = iter.reference_definitions();
    let mut found = Vec::new();
    for (i, (event, range)) in events.iter().enumerate() {
        let Event::Start(tag) = event else { continue };
        let (link_type, id, image) = match tag {
            Tag::Link { link_type, id, .. } => (*link_type, id, false),
            Tag::Image { link_type, id, .. } => (*link_type, id, true),
            _ => continue,
        };
        let dest_range = match link_type {
            LinkType::Inline => inline_dest(source, &events, i, range, image),
            LinkType::Autolink | LinkType::Email => {
                (range.end > range.start + 1).then(|| range.start + 1..range.end - 1)
            }
            LinkType::Reference
            | LinkType::Collapsed
            | LinkType::Shortcut
            | LinkType::ReferenceUnknown
            | LinkType::CollapsedUnknown
            | LinkType::ShortcutUnknown => {
                // O rótulo da definição: o `id` do `[a][1]`, ou o texto de `[a][]` e `[a]`.
                let label = if id.is_empty() {
                    plain_text(&events, i)
                } else {
                    id.to_string()
                };
                definitions
                    .get(&label)
                    .and_then(|definition| definition_dest(source, &definition.span))
            }
            LinkType::WikiLink { .. } => None,
        };
        let Some(dest_range) = dest_range else {
            continue;
        };
        found.push(LinkRef {
            dest: source[dest_range.clone()].to_owned(),
            dest_range,
            image,
        });
    }
    found
}

/// O texto dentro do link que começa no evento `start`.
fn plain_text(events: &[(Event<'_>, Range<usize>)], start: usize) -> String {
    let mut text = String::new();
    let mut depth = 0;
    for (event, _) in &events[start..] {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Event::Text(t) | Event::Code(t) => text.push_str(t),
            _ => {}
        }
    }
    text
}

/// O destino de um link ou imagem em linha, `[texto](destino "título")`.
fn inline_dest(
    source: &str,
    events: &[(Event<'_>, Range<usize>)],
    start: usize,
    range: &Range<usize>,
    image: bool,
) -> Option<Range<usize>> {
    // Onde o texto do link acaba: o fim do último evento de dentro.
    let mut depth = 0;
    let mut after_text = range.start + if image { 2 } else { 1 };
    for (event, event_range) in &events[start + 1..] {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) if depth == 0 => break,
            Event::End(_) => depth -= 1,
            _ => {}
        }
        after_text = after_text.max(event_range.end);
    }
    let after_text = after_text.min(range.end);
    let open = after_text + source.get(after_text..range.end)?.find('(')?;
    let close = range.end.checked_sub(1).filter(|&c| c > open)?;
    destination(source, open + 1, close)
}

/// O destino de uma definição `[rótulo]: destino "título"`.
fn definition_dest(source: &str, span: &Range<usize>) -> Option<Range<usize>> {
    let text = source.get(span.clone())?;
    let colon = text.find("]:")?;
    destination(source, span.start + colon + 2, span.end)
}

/// A faixa do destino em `source[from..to]`: o que vem entre `<` e `>`, ou até o primeiro
/// espaço.
fn destination(source: &str, from: usize, to: usize) -> Option<Range<usize>> {
    let region = source.get(from..to)?;
    let skipped = region.len() - region.trim_start().len();
    let start = from + skipped;
    let rest = &source[start..to];
    if let Some(inner) = rest.strip_prefix('<') {
        let mut escaped = false;
        for (i, c) in inner.char_indices() {
            match c {
                '\\' if !escaped => escaped = true,
                '>' if !escaped => return Some(start + 1..start + 1 + i),
                _ => escaped = false,
            }
        }
        return None;
    }
    let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
    (len > 0).then(|| start..start + len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destinos(source: &str) -> Vec<(String, bool, String)> {
        links(source)
            .into_iter()
            .map(|l| (l.dest, l.image, source[l.dest_range].to_owned()))
            .collect()
    }

    #[test]
    fn link_imagem_e_titulo() {
        let achados = destinos("[a](um.md) ![b](dois.png) [c](tres.md \"título\")");
        assert_eq!(
            achados,
            vec![
                ("um.md".into(), false, "um.md".into()),
                ("dois.png".into(), true, "dois.png".into()),
                ("tres.md".into(), false, "tres.md".into()),
            ]
        );
    }

    #[test]
    fn destino_entre_menor_e_maior_fica_sem_eles() {
        let texto = "[a](<com espaço.md>) e [b](<x>)";
        let achados = links(texto);
        assert_eq!(achados[0].dest, "com espaço.md");
        assert_eq!(&texto[achados[0].dest_range.clone()], "com espaço.md");
        assert_eq!(achados[1].dest, "x");
    }

    #[test]
    fn referencia_aponta_para_a_definicao() {
        let texto = "Veja [a][1] e [b][] e [c].\n\n[1]: http://um\n[b]: <http://dois> \"t\"\n[c]: http://tres\n";
        let achados = links(texto);
        assert_eq!(achados.len(), 3);
        assert_eq!(achados[0].dest, "http://um");
        assert_eq!(achados[1].dest, "http://dois");
        assert_eq!(achados[2].dest, "http://tres");
        for achado in &achados {
            assert_eq!(&texto[achado.dest_range.clone()], achado.dest);
        }
        assert!(achados[0].dest_range.start > texto.find("\n\n").unwrap());
    }

    #[test]
    fn codigo_nao_conta() {
        let texto = "`[x](y)`\n\n```\n[x](y)\n```\n\n[z](w)";
        let achados = links(texto);
        assert_eq!(achados.len(), 1);
        assert_eq!(achados[0].dest, "w");
    }

    #[test]
    fn link_com_imagem_dentro_e_autolink() {
        let texto = "[![logo](l.png)](https://site) <https://auto>";
        let achados = destinos(texto);
        assert_eq!(achados.len(), 3);
        assert_eq!(achados[0].0, "https://site");
        assert_eq!(achados[1].0, "l.png");
        assert!(achados[1].1);
        assert_eq!(achados[2].0, "https://auto");
    }

    #[test]
    fn texto_sem_links() {
        assert!(links("nada de links aqui").is_empty());
        assert!(links("").is_empty());
    }
}
