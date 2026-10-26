//! Caminhos do [`Fs`](crate::Fs): texto com `/`, absoluto e normalizado. No Windows, o
//! [`Local`](crate::Local) mostra `C:/pasta` (a letra da unidade fica como está).

/// O prefixo de unidade (`C:`) de um caminho do Windows, se tiver.
fn drive(path: &str) -> Option<&str> {
    let bytes = path.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':').then(|| &path[..2])
}

/// Normaliza: troca `\` por `/`, junta barras repetidas, tira `.` e resolve `..` no texto.
/// O resultado começa com `/` (ou com a unidade, `C:/`) e não termina com `/`, exceto na raiz.
///
/// ```
/// use tomni_ui_files::path::normalize;
///
/// assert_eq!(normalize("/a//b/./c/../d/"), "/a/b/d");
/// assert_eq!(normalize(""), "/");
/// assert_eq!(normalize("/.."), "/");
/// ```
pub fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let (prefix, rest) = match drive(&path) {
        Some(d) => (d.to_owned(), &path[2..]),
        None => (String::new(), path.as_str()),
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        format!("{prefix}/")
    } else {
        format!("{prefix}/{}", parts.join("/"))
    }
}

/// Junta `name` a `base`. Um `name` absoluto substitui a base.
///
/// ```
/// use tomni_ui_files::path::join;
///
/// assert_eq!(join("/home/ana", "docs/a.txt"), "/home/ana/docs/a.txt");
/// assert_eq!(join("/home/ana", "/etc"), "/etc");
/// assert_eq!(join("/home/ana", ".."), "/home");
/// ```
pub fn join(base: &str, name: &str) -> String {
    let name = name.replace('\\', "/");
    if name.starts_with('/') || drive(&name).is_some() {
        normalize(&name)
    } else {
        normalize(&format!("{}/{name}", base.trim_end_matches('/')))
    }
}

/// A pasta que contém `path` (a raiz é pai de si mesma).
///
/// ```
/// use tomni_ui_files::path::parent;
///
/// assert_eq!(parent("/a/b"), "/a");
/// assert_eq!(parent("/a"), "/");
/// assert_eq!(parent("/"), "/");
/// ```
pub fn parent(path: &str) -> String {
    let path = normalize(path);
    match path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        // `C:/x` → `C:/`.
        Some(2) if drive(&path).is_some() => path[..3].to_owned(),
        Some(i) => path[..i].to_owned(),
    }
}

/// O último componente de `path` (vazio na raiz).
///
/// ```
/// use tomni_ui_files::path::file_name;
///
/// assert_eq!(file_name("/a/b.txt"), "b.txt");
/// assert_eq!(file_name("/"), "");
/// ```
pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or_default();
    if drive(name).is_some() && name.len() == 2 {
        ""
    } else {
        name
    }
}

/// O nome com ` (n)` antes da extensão: `relatório.pdf` vira `relatório (1).pdf`. Um nome que
/// começa com ponto (`.bashrc`) e um sem extensão levam o sufixo no fim.
///
/// ```
/// use tomni_ui_files::path::numbered;
///
/// assert_eq!(numbered("a.txt", 2), "a (2).txt");
/// assert_eq!(numbered(".bashrc", 1), ".bashrc (1)");
/// assert_eq!(numbered("LEIA", 1), "LEIA (1)");
/// ```
pub fn numbered(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({n}){}", &name[..i], &name[i..]),
        _ => format!("{name} ({n})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normaliza_unidades_do_windows() {
        assert_eq!(normalize("C:\\Users\\ana\\..\\bia"), "C:/Users/bia");
        assert_eq!(normalize("C:"), "C:/");
        assert_eq!(parent("C:/Users"), "C:/");
        assert_eq!(parent("C:/"), "C:/");
        assert_eq!(join("C:/Users", "ana"), "C:/Users/ana");
        assert_eq!(join("/x", "D:/y"), "D:/y");
        assert_eq!(file_name("C:/"), "");
    }

    #[test]
    fn nomes_com_acentos_e_espacos_passam_inteiros() {
        assert_eq!(
            join("/dados", "relatório final.pdf"),
            "/dados/relatório final.pdf"
        );
        assert_eq!(
            file_name("/dados/relatório final.pdf"),
            "relatório final.pdf"
        );
        assert_eq!(
            numbered("relatório final.pdf", 1),
            "relatório final (1).pdf"
        );
    }

    #[test]
    fn dois_pontos_acima_da_raiz_ficam_na_raiz() {
        assert_eq!(join("/a", "../../.."), "/");
    }
}
