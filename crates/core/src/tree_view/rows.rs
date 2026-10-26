//! As linhas visíveis de um `TreeView`: os itens achatados, com o filtro.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::model::{TreeItem, fold};
use crate::symbol::Symbol;

/// Uma linha visível.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    /// Chave do item.
    pub key: String,
    /// Rótulo.
    pub label: String,
    /// Ícone já resolvido (o do item, ou pasta aberta/fechada e documento).
    pub icon: Symbol,
    /// O selo do item, depois do rótulo.
    pub badge: Option<Symbol>,
    /// Nível: 0 na raiz.
    pub depth: usize,
    /// Se o item é uma pasta.
    pub folder: bool,
    /// Se mostra o chevron: é pasta com filhos (com filtro, com filhos que aparecem).
    pub has_children: bool,
    /// Se a pasta aparece aberta agora (com filtro, pelo estado do filtro).
    pub expanded: bool,
    /// Chave do pai; `None` na raiz.
    pub parent: Option<String>,
    /// Posição entre os irmãos do item, nos itens (sem o filtro).
    pub index: usize,
}

/// As linhas visíveis em ordem, mais o índice de cada chave.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeFlat {
    /// As linhas.
    pub rows: Vec<TreeRow>,
    index: HashMap<String, usize>,
}

impl TreeFlat {
    /// A posição da chave nas linhas visíveis.
    pub fn position(&self, key: &str) -> Option<usize> {
        self.index.get(key).copied()
    }

    /// A linha da chave, se aparece.
    pub fn get(&self, key: &str) -> Option<&TreeRow> {
        self.position(key).map(|i| &self.rows[i])
    }
}

/// Acha as linhas visíveis de `items`.
///
/// Sem filtro (texto vazio), entra só nas pastas de `expanded`. Com filtro, aparecem os itens
/// cujo rótulo (ou `search_text`) contém o texto (sem diferença de maiúsculas e acentos) e os
/// ancestrais deles,
/// abertos, menos os de `filter_collapsed`. Uma pasta que casa mostra só os filhos que também
/// aparecem.
pub fn flatten(
    items: &[TreeItem],
    expanded: &BTreeSet<String>,
    filter: &str,
    filter_collapsed: &BTreeSet<String>,
) -> TreeFlat {
    #[cfg(debug_assertions)]
    check_keys(items, &mut HashSet::new());
    let needle = fold(filter.trim());
    let mut rows = Vec::new();
    if needle.is_empty() {
        walk(items, 0, None, expanded, &mut rows);
    } else {
        let visible = matching(items, &needle);
        walk_filtered(items, 0, None, &visible, filter_collapsed, &mut rows);
    }
    let index = rows
        .iter()
        .enumerate()
        .map(|(i, row)| (row.key.clone(), i))
        .collect();
    TreeFlat { rows, index }
}

#[cfg(debug_assertions)]
fn check_keys<'a>(items: &'a [TreeItem], seen: &mut HashSet<&'a str>) {
    for item in items {
        assert!(
            seen.insert(&item.key),
            "chave repetida no TreeView: {:?} (as chaves são únicas na árvore inteira)",
            item.key
        );
        check_keys(&item.children, seen);
    }
}

fn icon_of(item: &TreeItem, expanded: bool) -> Symbol {
    item.icon.unwrap_or(match (item.folder, expanded) {
        (true, true) => Symbol::FolderOpen,
        (true, false) => Symbol::Folder,
        (false, _) => Symbol::Document,
    })
}

fn walk(
    items: &[TreeItem],
    depth: usize,
    parent: Option<&str>,
    expanded: &BTreeSet<String>,
    rows: &mut Vec<TreeRow>,
) {
    for (index, item) in items.iter().enumerate() {
        let has_children = !item.children.is_empty();
        let open = has_children && expanded.contains(&item.key);
        rows.push(TreeRow {
            key: item.key.clone(),
            label: item.label.clone(),
            icon: icon_of(item, open),
            badge: item.badge,
            depth,
            folder: item.folder,
            has_children,
            expanded: open,
            parent: parent.map(str::to_owned),
            index,
        });
        if open {
            walk(&item.children, depth + 1, Some(&item.key), expanded, rows);
        }
    }
}

/// As chaves que aparecem com o filtro: as que casam e os ancestrais delas.
fn matching(items: &[TreeItem], needle: &str) -> HashSet<String> {
    fn visit(items: &[TreeItem], needle: &str, out: &mut HashSet<String>) -> bool {
        let mut any = false;
        for item in items {
            let below = visit(&item.children, needle, out);
            let found = fold(&item.label).contains(needle)
                || (!item.search_text.is_empty() && fold(&item.search_text).contains(needle));
            if below || found {
                out.insert(item.key.clone());
                any = true;
            }
        }
        any
    }
    let mut out = HashSet::new();
    visit(items, needle, &mut out);
    out
}

fn walk_filtered(
    items: &[TreeItem],
    depth: usize,
    parent: Option<&str>,
    visible: &HashSet<String>,
    collapsed: &BTreeSet<String>,
    rows: &mut Vec<TreeRow>,
) {
    for (index, item) in items.iter().enumerate() {
        if !visible.contains(&item.key) {
            continue;
        }
        let has_children = item.children.iter().any(|c| visible.contains(&c.key));
        let open = has_children && !collapsed.contains(&item.key);
        rows.push(TreeRow {
            key: item.key.clone(),
            label: item.label.clone(),
            icon: icon_of(item, open),
            badge: item.badge,
            depth,
            folder: item.folder,
            has_children,
            expanded: open,
            parent: parent.map(str::to_owned),
            index,
        });
        if open {
            walk_filtered(
                &item.children,
                depth + 1,
                Some(&item.key),
                visible,
                collapsed,
                rows,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn itens() -> Vec<TreeItem> {
        vec![
            TreeItem::folder("fav", "Favoritos")
                .child(
                    TreeItem::folder("trab", "Trabalho")
                        .child(
                            TreeItem::folder("srv", "Servidores")
                                .child(TreeItem::leaf("web", "web-01")),
                        )
                        .child(TreeItem::leaf("wiki", "Wiki")),
                )
                .child(TreeItem::leaf("not", "Notícias")),
            TreeItem::folder("acoes", "Ações").child(TreeItem::leaf("bolsa", "Bolsa")),
            TreeItem::leaf("leitura", "Leitura"),
        ]
    }

    fn set(chaves: &[&str]) -> BTreeSet<String> {
        chaves.iter().map(|c| (*c).to_owned()).collect()
    }

    fn resumo(flat: &TreeFlat) -> String {
        flat.rows
            .iter()
            .map(|r| {
                let marca = match (r.has_children, r.expanded) {
                    (false, _) => "",
                    (true, true) => "-",
                    (true, false) => "+",
                };
                format!("{}{}{marca}", " ".repeat(r.depth), r.key)
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    #[test]
    fn so_entra_nas_pastas_abertas() {
        let flat = flatten(&itens(), &set(&[]), "", &set(&[]));
        assert_eq!(resumo(&flat), "fav+|acoes+|leitura");
        let flat = flatten(&itens(), &set(&["fav"]), "", &set(&[]));
        assert_eq!(resumo(&flat), "fav-| trab+| not|acoes+|leitura");
        let flat = flatten(&itens(), &set(&["fav", "trab", "srv"]), "", &set(&[]));
        assert_eq!(
            resumo(&flat),
            "fav-| trab-|  srv-|   web|  wiki| not|acoes+|leitura"
        );
    }

    #[test]
    fn selo_do_item_chega_na_linha() {
        let itens = vec![
            TreeItem::leaf("a", "A").badge(Symbol::Sparkle),
            TreeItem::leaf("b", "B"),
        ];
        let flat = flatten(&itens, &set(&[]), "", &set(&[]));
        assert_eq!(flat.get("a").unwrap().badge, Some(Symbol::Sparkle));
        assert_eq!(flat.get("b").unwrap().badge, None);
    }

    #[test]
    fn pasta_aberta_no_conjunto_mas_dentro_de_fechada_nao_aparece() {
        let flat = flatten(&itens(), &set(&["srv"]), "", &set(&[]));
        assert_eq!(resumo(&flat), "fav+|acoes+|leitura");
    }

    #[test]
    fn preenche_nivel_pai_posicao_e_icone() {
        let flat = flatten(&itens(), &set(&["fav", "trab"]), "", &set(&[]));
        let wiki = flat.get("wiki").unwrap();
        assert_eq!(wiki.depth, 2);
        assert_eq!(wiki.parent.as_deref(), Some("trab"));
        assert_eq!(wiki.index, 1);
        assert_eq!(wiki.icon, Symbol::Document);
        assert_eq!(flat.get("fav").unwrap().icon, Symbol::FolderOpen);
        assert_eq!(flat.get("acoes").unwrap().icon, Symbol::Folder);
        assert_eq!(flat.get("fav").unwrap().parent, None);
        assert_eq!(flat.position("not"), Some(4));
        assert_eq!(flat.position("web"), None);
    }

    #[test]
    fn icone_proprio_vale() {
        let itens = vec![TreeItem::leaf("a", "A").icon(Symbol::Home)];
        let flat = flatten(&itens, &set(&[]), "", &set(&[]));
        assert_eq!(flat.rows[0].icon, Symbol::Home);
    }

    #[test]
    fn pasta_vazia_nao_tem_chevron() {
        let itens = vec![TreeItem::folder("vazia", "Vazia")];
        let flat = flatten(&itens, &set(&["vazia"]), "", &set(&[]));
        assert!(flat.rows[0].folder);
        assert!(!flat.rows[0].has_children);
        assert!(!flat.rows[0].expanded);
    }

    #[test]
    fn filtro_mantem_os_ancestrais_abertos() {
        let flat = flatten(&itens(), &set(&[]), "web", &set(&[]));
        assert_eq!(resumo(&flat), "fav-| trab-|  srv-|   web");
    }

    #[test]
    fn filtro_procura_tambem_no_texto_de_busca() {
        let itens = vec![
            TreeItem::folder("p", "Produção")
                .child(TreeItem::leaf("a", "srv-a").search_text("Servidor do ERP"))
                .child(TreeItem::leaf("b", "srv-b")),
        ];
        let flat = flatten(&itens, &set(&[]), "erp", &set(&[]));
        assert_eq!(resumo(&flat), "p-| a");
        assert_eq!(flat.rows[1].label, "srv-a", "o texto de busca não aparece");
    }

    #[test]
    fn filtro_ignora_maiusculas_e_acentos() {
        let flat = flatten(&itens(), &set(&[]), "ACOE", &set(&[]));
        assert_eq!(resumo(&flat), "acoes");
        let flat = flatten(&itens(), &set(&[]), "noticias", &set(&[]));
        assert_eq!(resumo(&flat), "fav-| not");
    }

    #[test]
    fn pasta_que_casa_mostra_so_os_filhos_que_tambem_aparecem() {
        let flat = flatten(&itens(), &set(&[]), "trabalho", &set(&[]));
        assert_eq!(resumo(&flat), "fav-| trab");
    }

    #[test]
    fn filtro_respeita_o_que_o_usuario_fechou() {
        let flat = flatten(&itens(), &set(&[]), "web", &set(&["trab"]));
        assert_eq!(resumo(&flat), "fav-| trab+");
        assert!(!flat.get("trab").unwrap().expanded);
    }

    #[test]
    fn filtro_vazio_ou_so_espacos_nao_filtra() {
        let a = flatten(&itens(), &set(&["fav"]), "", &set(&[]));
        let b = flatten(&itens(), &set(&["fav"]), "   ", &set(&[]));
        assert_eq!(a, b);
    }

    #[test]
    fn filtro_sem_resultado_deixa_a_lista_vazia() {
        let flat = flatten(&itens(), &set(&[]), "zzz", &set(&[]));
        assert!(flat.rows.is_empty());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "chave repetida no TreeView")]
    fn chave_repetida_e_erro_em_debug() {
        let itens = vec![
            TreeItem::leaf("a", "A"),
            TreeItem::folder("p", "P").child(TreeItem::leaf("a", "A2")),
        ];
        let _ = flatten(&itens, &set(&[]), "", &set(&[]));
    }
}
