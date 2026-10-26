//! Um leitor de XML mínimo para as respostas do S3.
//!
//! O `rusty-s3` lê as respostas só com o `xmlns` da AWS; alguns servidores compatíveis o omitem.
//! Aqui o espaço de nomes não conta: só os nomes locais dos elementos.

use quick_xml::Reader;
use quick_xml::events::Event;

/// Um elemento: o nome, o texto direto e os filhos.
#[derive(Debug, Default, Clone)]
pub(crate) struct Node {
    pub name: String,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    /// O primeiro filho com este nome.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    /// Os filhos com este nome.
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// O texto do primeiro filho com este nome.
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|child| child.text.as_str())
    }
}

/// Lê o documento e devolve o elemento raiz, ou `None` se não for XML.
pub(crate) fn parse(xml: &str) -> Option<Node> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    loop {
        match reader.read_event().ok()? {
            Event::Start(start) => stack.push(Node {
                name: String::from_utf8_lossy(start.local_name().as_ref()).into_owned(),
                ..Node::default()
            }),
            Event::Empty(start) => {
                let node = Node {
                    name: String::from_utf8_lossy(start.local_name().as_ref()).into_owned(),
                    ..Node::default()
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            Event::End(_) => {
                let node = stack.pop()?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            Event::Text(text) => {
                if let (Some(top), Ok(text)) = (stack.last_mut(), text.decode()) {
                    top.text.push_str(&text);
                }
            }
            Event::CData(data) => {
                if let (Some(top), Ok(text)) = (stack.last_mut(), data.decode()) {
                    top.text.push_str(&text);
                }
            }
            Event::GeneralRef(reference) => {
                let Some(top) = stack.last_mut() else {
                    continue;
                };
                if let Ok(Some(character)) = reference.resolve_char_ref() {
                    top.text.push(character);
                } else if let Ok(name) = reference.decode() {
                    match name.as_ref() {
                        "lt" => top.text.push('<'),
                        "gt" => top.text.push('>'),
                        "amp" => top.text.push('&'),
                        "quot" => top.text.push('"'),
                        "apos" => top.text.push('\''),
                        _ => {}
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.is_empty() { root } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_sem_espaco_de_nomes_e_com_entidades() {
        let xml = r#"<?xml version="1.0"?><ListAllMyBucketsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Buckets><Bucket><Name>a&amp;b</Name></Bucket><Bucket><Name>c</Name></Bucket></Buckets><Vazio/></ListAllMyBucketsResult>"#;
        let raiz = parse(xml).unwrap();
        assert_eq!(raiz.name, "ListAllMyBucketsResult");
        let nomes: Vec<_> = raiz
            .child("Buckets")
            .unwrap()
            .all("Bucket")
            .map(|bucket| bucket.text_of("Name").unwrap())
            .collect();
        assert_eq!(nomes, ["a&b", "c"]);
        assert!(raiz.child("Vazio").is_some());
        assert!(parse("não é xml <").is_none());
    }
}
