//! Os itens de um `TreeView` e as operações que o app faz neles.

use crate::symbol::Symbol;

/// Um item do `TreeView`: uma pasta (que pode ter filhos) ou uma folha.
///
/// A `key` identifica o item na árvore inteira e é o que o `TreeView` guarda na seleção, nas
/// pastas abertas e no id dos itens (`{id}-{key}`). Uma chave repetida é um erro do app
/// (`panic` em debug, com a chave na mensagem).
///
/// ```
/// use tomni_ui_core::TreeItem;
///
/// let favoritos = TreeItem::folder("fav", "Favoritos")
///     .child(TreeItem::leaf("wiki", "Wiki"))
///     .child(TreeItem::folder("trabalho", "Trabalho").icon(tomni_ui_core::Symbol::Briefcase));
/// assert_eq!(favoritos.children.len(), 2);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct TreeItem {
    /// Chave única na árvore inteira.
    pub key: String,
    /// Rótulo mostrado.
    pub label: String,
    /// Ícone próprio; sem ele, a pasta mostra `Folder`/`FolderOpen` e a folha, `Document`.
    pub icon: Option<Symbol>,
    /// Selo depois do rótulo (um ícone pequeno, na cor secundária): marca que o item tem algo
    /// a mais, como uma permissão própria.
    pub badge: Option<Symbol>,
    /// Se é uma pasta (aceita filhos e itens soltos dentro).
    pub folder: bool,
    /// Os filhos, numa pasta.
    pub children: Vec<TreeItem>,
    /// Texto que o filtro também procura, sem aparecer na linha (uma descrição, apelidos).
    pub search_text: String,
}

impl TreeItem {
    /// Uma pasta, sem filhos ainda.
    pub fn folder(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            icon: None,
            badge: None,
            folder: true,
            children: Vec::new(),
            search_text: String::new(),
        }
    }

    /// Uma folha.
    pub fn leaf(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            icon: None,
            badge: None,
            folder: false,
            children: Vec::new(),
            search_text: String::new(),
        }
    }

    /// Os filhos de uma pasta. Numa folha é um erro (`panic` em debug).
    pub fn children(mut self, children: impl IntoIterator<Item = TreeItem>) -> Self {
        debug_assert!(
            self.folder,
            "`children` numa folha do TreeView: {:?} (use `TreeItem::folder`)",
            self.key
        );
        self.children = children.into_iter().collect();
        self
    }

    /// Acrescenta um filho a uma pasta. Numa folha é um erro (`panic` em debug).
    pub fn child(mut self, child: TreeItem) -> Self {
        debug_assert!(
            self.folder,
            "`child` numa folha do TreeView: {:?} (use `TreeItem::folder`)",
            self.key
        );
        self.children.push(child);
        self
    }

    /// Troca o ícone padrão.
    pub fn icon(mut self, icon: Symbol) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Põe um selo depois do rótulo: um ícone pequeno na cor secundária.
    pub fn badge(mut self, badge: Symbol) -> Self {
        self.badge = Some(badge);
        self
    }

    /// Um texto que o `filter` do `TreeView` também procura, além do rótulo, sem mostrar.
    pub fn search_text(mut self, text: impl Into<String>) -> Self {
        self.search_text = text.into();
        self
    }

    /// Se `key` é este item ou um descendente dele.
    fn contains(&self, key: &str) -> bool {
        self.key == key || self.children.iter().any(|c| c.contains(key))
    }
}

/// As operações que o app faz nos itens de um `TreeView`, em `Vec<TreeItem>`:
/// `itens.update(|v| { v.move_to("a", Some("pasta"), 0); })`.
///
/// `insert_item` e `remove_item` não se chamam `insert` e `remove` porque o `Vec` tem métodos
/// próprios com esses nomes, que ganhariam na resolução.
///
/// Os itens são identificados pela chave, e a seleção e as pastas abertas do `TreeView` também
/// guardam chaves: todas estas operações preservam os dois.
pub trait TreeItems {
    /// O item com a chave `key`, em qualquer nível.
    fn find(&self, key: &str) -> Option<&TreeItem>;

    /// Como [`find`](Self::find), para mudar o item.
    fn find_mut(&mut self, key: &str) -> Option<&mut TreeItem>;

    /// O pai do item `key`: `None` se a chave não existe; `Some(None)` se o item está na raiz.
    fn parent_of(&self, key: &str) -> Option<Option<&str>>;

    /// Insere `item` em `parent` (`None` é a raiz) na posição `position`, limitada ao tamanho
    /// da lista. Devolve `false`, sem mudar nada, se o pai não existe ou não é pasta, ou se
    /// alguma chave de `item` (ou dos filhos dele) já existe.
    fn insert_item(&mut self, parent: Option<&str>, position: usize, item: TreeItem) -> bool;

    /// Tira o item `key` (com os filhos) e o devolve.
    fn remove_item(&mut self, key: &str) -> Option<TreeItem>;

    /// Move o item `key` para `parent` (`None` é a raiz). `position` é o índice na lista final
    /// do pai, depois de tirar o item: é o que o `on_move` do `TreeView` entrega. Devolve
    /// `false`, sem mudar nada, se a chave não existe, se o destino é o próprio item ou um
    /// descendente dele, ou se o pai não existe ou não é pasta.
    fn move_to(&mut self, key: &str, parent: Option<&str>, position: usize) -> bool;

    /// Troca o item com o irmão de cima. `false` se já é o primeiro (ou não existe).
    fn move_up(&mut self, key: &str) -> bool;

    /// Troca o item com o irmão de baixo. `false` se já é o último (ou não existe).
    fn move_down(&mut self, key: &str) -> bool;

    /// Troca o rótulo do item. `false` se a chave não existe.
    fn rename(&mut self, key: &str, label: impl Into<String>) -> bool;
}

/// A lista de irmãos (a raiz ou os filhos de uma pasta) que contém `key`, e a posição dele.
fn siblings_of<'a>(
    items: &'a mut Vec<TreeItem>,
    key: &str,
) -> Option<(&'a mut Vec<TreeItem>, usize)> {
    if let Some(i) = items.iter().position(|item| item.key == key) {
        return Some((items, i));
    }
    for item in items.iter_mut() {
        if let Some(found) = siblings_of(&mut item.children, key) {
            return Some(found);
        }
    }
    None
}

fn parent_in<'a>(
    items: &'a [TreeItem],
    key: &str,
    parent: Option<&'a str>,
) -> Option<Option<&'a str>> {
    for item in items {
        if item.key == key {
            return Some(parent);
        }
        if let Some(found) = parent_in(&item.children, key, Some(&item.key)) {
            return Some(found);
        }
    }
    None
}

/// A lista de filhos de `parent`, ou a raiz. `None` se o pai não existe ou não é pasta.
fn children_of<'a>(
    items: &'a mut Vec<TreeItem>,
    parent: Option<&str>,
) -> Option<&'a mut Vec<TreeItem>> {
    match parent {
        None => Some(items),
        Some(key) => {
            let item = find_mut(items, key)?;
            item.folder.then_some(&mut item.children)
        }
    }
}

fn find_mut<'a>(items: &'a mut [TreeItem], key: &str) -> Option<&'a mut TreeItem> {
    for item in items {
        if item.key == key {
            return Some(item);
        }
        if let Some(found) = find_mut(&mut item.children, key) {
            return Some(found);
        }
    }
    None
}

fn find<'a>(items: &'a [TreeItem], key: &str) -> Option<&'a TreeItem> {
    for item in items {
        if item.key == key {
            return Some(item);
        }
        if let Some(found) = find(&item.children, key) {
            return Some(found);
        }
    }
    None
}

impl TreeItems for Vec<TreeItem> {
    fn find(&self, key: &str) -> Option<&TreeItem> {
        find(self, key)
    }

    fn find_mut(&mut self, key: &str) -> Option<&mut TreeItem> {
        find_mut(self, key)
    }

    fn parent_of(&self, key: &str) -> Option<Option<&str>> {
        parent_in(self, key, None)
    }

    fn insert_item(&mut self, parent: Option<&str>, position: usize, item: TreeItem) -> bool {
        let mut keys = Vec::new();
        collect_keys(&item, &mut keys);
        if keys.iter().any(|key| find(self, key).is_some()) {
            return false;
        }
        let Some(list) = children_of(self, parent) else {
            return false;
        };
        let position = position.min(list.len());
        list.insert(position, item);
        true
    }

    fn remove_item(&mut self, key: &str) -> Option<TreeItem> {
        let (list, i) = siblings_of(self, key)?;
        Some(list.remove(i))
    }

    fn move_to(&mut self, key: &str, parent: Option<&str>, position: usize) -> bool {
        let Some(item) = find(self, key) else {
            return false;
        };
        if parent.is_some_and(|p| item.contains(p)) {
            return false;
        }
        if let Some(p) = parent
            && !find(self, p).is_some_and(|p| p.folder)
        {
            return false;
        }
        let Some(item) = self.remove_item(key) else {
            return false;
        };
        let list =
            children_of(self, parent).expect("o destino foi conferido antes de tirar o item");
        list.insert(position.min(list.len()), item);
        true
    }

    fn move_up(&mut self, key: &str) -> bool {
        match siblings_of(self, key) {
            Some((list, i)) if i > 0 => {
                list.swap(i, i - 1);
                true
            }
            _ => false,
        }
    }

    fn move_down(&mut self, key: &str) -> bool {
        match siblings_of(self, key) {
            Some((list, i)) if i + 1 < list.len() => {
                list.swap(i, i + 1);
                true
            }
            _ => false,
        }
    }

    fn rename(&mut self, key: &str, label: impl Into<String>) -> bool {
        match find_mut(self, key) {
            Some(item) => {
                item.label = label.into();
                true
            }
            None => false,
        }
    }
}

fn collect_keys<'a>(item: &'a TreeItem, keys: &mut Vec<&'a str>) {
    keys.push(&item.key);
    for child in &item.children {
        collect_keys(child, keys);
    }
}

/// O texto sem maiúsculas e sem acentos, para comparar no filtro e na letra inicial.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(fold_char)
        .collect()
}

pub(crate) fn fold_char(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arvore() -> Vec<TreeItem> {
        vec![
            TreeItem::folder("a", "A")
                .child(TreeItem::leaf("a1", "A1"))
                .child(TreeItem::folder("a2", "A2").child(TreeItem::leaf("a21", "A21")))
                .child(TreeItem::leaf("a3", "A3")),
            TreeItem::leaf("b", "B"),
            TreeItem::folder("c", "C"),
        ]
    }

    fn chaves(itens: &[TreeItem]) -> String {
        fn percorre(itens: &[TreeItem], out: &mut Vec<String>) {
            for item in itens {
                if item.children.is_empty() {
                    out.push(item.key.clone());
                } else {
                    let mut filhos = Vec::new();
                    percorre(&item.children, &mut filhos);
                    out.push(format!("{}({})", item.key, filhos.join(" ")));
                }
            }
        }
        let mut out = Vec::new();
        percorre(itens, &mut out);
        out.join(" ")
    }

    #[test]
    fn acha_o_item_e_o_pai() {
        let v = arvore();
        assert_eq!(v.find("a21").unwrap().label, "A21");
        assert!(v.find("zzz").is_none());
        assert_eq!(v.parent_of("a21"), Some(Some("a2")));
        assert_eq!(v.parent_of("b"), Some(None));
        assert_eq!(v.parent_of("zzz"), None);
    }

    #[test]
    fn insere_na_raiz_e_em_pastas() {
        let mut v = arvore();
        assert!(v.insert_item(None, 1, TreeItem::leaf("n", "N")));
        assert!(v.insert_item(Some("c"), 99, TreeItem::leaf("c1", "C1")));
        assert!(v.insert_item(Some("a"), 0, TreeItem::leaf("a0", "A0")));
        assert_eq!(chaves(&v), "a(a0 a1 a2(a21) a3) n b c(c1)");
    }

    #[test]
    fn insere_recusa_pai_folha_inexistente_e_chave_repetida() {
        let mut v = arvore();
        assert!(
            !v.insert_item(Some("b"), 0, TreeItem::leaf("x", "X")),
            "pai folha"
        );
        assert!(
            !v.insert_item(Some("zzz"), 0, TreeItem::leaf("x", "X")),
            "pai inexistente"
        );
        assert!(
            !v.insert_item(None, 0, TreeItem::leaf("a21", "Outro")),
            "chave repetida"
        );
        let pasta = TreeItem::folder("p", "P").child(TreeItem::leaf("b", "B de novo"));
        assert!(!v.insert_item(None, 0, pasta), "chave repetida num filho");
        assert_eq!(chaves(&v), "a(a1 a2(a21) a3) b c");
    }

    #[test]
    fn remove_com_os_filhos() {
        let mut v = arvore();
        let tirado = v.remove_item("a2").unwrap();
        assert_eq!(tirado.children.len(), 1);
        assert_eq!(chaves(&v), "a(a1 a3) b c");
        assert!(v.remove_item("a2").is_none());
    }

    #[test]
    fn move_para_outro_pai() {
        let mut v = arvore();
        assert!(v.move_to("b", Some("a2"), 0));
        assert_eq!(chaves(&v), "a(a1 a2(b a21) a3) c");
        assert!(v.move_to("a21", None, 1));
        assert_eq!(chaves(&v), "a(a1 a2(b) a3) a21 c");
    }

    #[test]
    fn move_no_mesmo_pai_usa_a_lista_depois_de_tirar_o_item() {
        let mut v = arvore();
        // A lista sem "a1" é [a2 a3]; a posição 1 é entre elas.
        assert!(v.move_to("a1", Some("a"), 1));
        assert_eq!(chaves(&v), "a(a2(a21) a1 a3) b c");
        assert!(v.move_to("a1", Some("a"), 99));
        assert_eq!(chaves(&v), "a(a2(a21) a3 a1) b c");
        assert!(v.move_to("a1", Some("a"), 0));
        assert_eq!(chaves(&v), "a(a1 a2(a21) a3) b c");
    }

    #[test]
    fn move_recusa_si_descendente_folha_e_inexistente() {
        let mut v = arvore();
        let antes = v.clone();
        assert!(!v.move_to("a", Some("a"), 0), "dentro de si");
        assert!(!v.move_to("a", Some("a21"), 0), "dentro de um descendente");
        assert!(!v.move_to("a", Some("a2"), 0), "dentro de um filho");
        assert!(!v.move_to("b", Some("a1"), 0), "dentro de uma folha");
        assert!(!v.move_to("b", Some("zzz"), 0), "pai inexistente");
        assert!(!v.move_to("zzz", None, 0), "chave inexistente");
        assert_eq!(v, antes);
    }

    #[test]
    fn sobe_e_desce_entre_irmaos() {
        let mut v = arvore();
        assert!(v.move_up("a3"));
        assert_eq!(chaves(&v), "a(a1 a3 a2(a21)) b c");
        assert!(v.move_down("a1"));
        assert_eq!(chaves(&v), "a(a3 a1 a2(a21)) b c");
        assert!(!v.move_up("a"), "já é o primeiro");
        assert!(!v.move_down("c"), "já é o último");
        assert!(!v.move_up("zzz"));
    }

    #[test]
    fn renomeia() {
        let mut v = arvore();
        assert!(v.rename("a21", "Novo nome"));
        assert_eq!(v.find("a21").unwrap().label, "Novo nome");
        assert!(!v.rename("zzz", "x"));
    }

    #[test]
    fn fold_ignora_maiusculas_e_acentos() {
        assert_eq!(fold("Ação"), "acao");
        assert_eq!(fold("ÁGUA-Ç"), "agua-c");
        assert_eq!(fold("Notícias 2"), "noticias 2");
    }

    #[test]
    #[should_panic(expected = "`children` numa folha")]
    #[cfg(debug_assertions)]
    fn filhos_numa_folha_e_erro() {
        let _ = TreeItem::leaf("a", "A").children([TreeItem::leaf("b", "B")]);
    }
}
