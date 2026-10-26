//! `TreeView`: árvore de pastas e itens. `cargo run --example tree_view`.
//!
//! Os itens ficam num `Signal<Vec<TreeItem>>` e a seleção, num `Signal<Option<String>>`; o app
//! muda a lista com os métodos de [`TreeItems`] (`insert_item`, `remove_item`, `move_to`,
//! `move_up`, `move_down`, `rename`). O clique seleciona; o clique duplo e o Enter chamam
//! `.on_invoke` (e abrem ou fecham a pasta); F2 renomeia (`.renaming` deixa o app começar a
//! renomear), Delete chama `.on_delete` e arrastar uma linha chama `.on_move`. `.filter` mostra
//! só o que combina, com os ancestrais. `.on_grid_drop(grupo, ..)` recebe linhas de um
//! `DataGrid` do mesmo `.drag_group` soltas sobre uma pasta (`.can_grid_drop` recusa pastas); no
//! exemplo, os "Arquivos soltos" caem nas pastas. `.item_status` põe o selo de estado no canto
//! do ícone (aqui, o web-01 está conectado e o web-02 caiu) e `.item_tooltip` dá a dica da linha.
//! As linhas têm os ids `{id}-{chave}`. No `TestApp`:
//! `app.tree_items("servidores")`, `app.expand("servidores", "producao")` e `app.collapse(...)`.

use tomni_ui::prelude::*;

/// Os servidores do usuário, em pastas.
fn servidores_iniciais() -> Vec<TreeItem> {
    vec![
        TreeItem::folder("producao", "Produção")
            .child(TreeItem::leaf("web-01", "web-01").icon(Symbol::Server))
            .child(TreeItem::leaf("web-02", "web-02").icon(Symbol::Server))
            .child(TreeItem::leaf("db-01", "db-01").icon(Symbol::Database)),
        TreeItem::folder("homologacao", "Homologação")
            .child(TreeItem::leaf("hml-01", "hml-01").icon(Symbol::Server)),
        TreeItem::folder("backup", "Backup"),
        TreeItem::leaf("bastion", "bastion").icon(Symbol::Desktop),
    ]
}

/// Organizador de servidores: filtro em cima, a árvore e, embaixo, os botões e o status.
pub fn servidores() -> impl View {
    let itens = signal(servidores_iniciais());
    let selecionado = signal(None::<String>);
    let aberto = signal(std::collections::BTreeSet::from(["producao".to_owned()]));
    let filtro = signal(String::new());
    let renomeando = signal(None::<String>);
    let status = signal("Pronto".to_owned());
    let contador = signal(0_u32);
    let soltos = signal(vec![
        DataGridRow::new("ata.pdf").cell("ata.pdf", CellValue::Text("ata.pdf".to_owned())),
        DataGridRow::new("chaves.txt").cell("chaves.txt", CellValue::Text("chaves.txt".to_owned())),
    ]);
    let soltos_escolhidos = signal(std::collections::BTreeSet::<String>::new());

    // Onde um item novo entra: dentro da pasta selecionada ou ao lado do item selecionado.
    let destino = move || -> (Option<String>, usize) {
        let Some(chave) = selecionado.get_untracked() else {
            return (None, usize::MAX);
        };
        itens.with_untracked(|v| match v.find(&chave) {
            Some(item) if item.folder => (Some(chave.clone()), usize::MAX),
            Some(_) => (v.parent_of(&chave).flatten().map(str::to_owned), usize::MAX),
            None => (None, usize::MAX),
        })
    };
    let novo = move |pasta: bool| {
        contador.update(|n| *n += 1);
        let chave = format!("novo-{}", contador.get_untracked());
        let (pai, posicao) = destino();
        let item = if pasta {
            TreeItem::folder(chave.clone(), "Nova pasta")
        } else {
            TreeItem::leaf(chave.clone(), "Novo servidor").icon(Symbol::Server)
        };
        let mut inserido = false;
        itens.update(|v| inserido = v.insert_item(pai.as_deref(), posicao, item));
        if inserido {
            if let Some(pai) = pai {
                aberto.update(|set| {
                    set.insert(pai);
                });
            }
            selecionado.set(Some(chave.clone()));
            renomeando.set(Some(chave));
            status.set("Digite o nome e tecle Enter".to_owned());
        }
    };
    let apagar = move |chave: String| {
        let mut apagado = None;
        itens.update(|v| apagado = v.remove_item(&chave));
        if let Some(item) = apagado {
            status.set(format!("{} apagado", item.label));
        }
    };

    Form::new("Servidores").child(
        Column::new()
            .grow(1)
            .padding(12)
            .gap(8)
            .child(
                Edit::bind(filtro)
                    .id("filtro")
                    .height(44)
                    .placeholder("Filtrar servidores"),
            )
            .child(
                TreeView::bind(itens, selecionado)
                    .id("servidores")
                    .grow(1)
                    .expanded(aberto)
                    .filter(filtro)
                    .renaming(renomeando)
                    .item_status(|chave| match chave {
                        "web-01" => Some(ItemStatus::Success),
                        "web-02" => Some(ItemStatus::Critical),
                        _ => None,
                    })
                    .item_tooltip(|chave| {
                        (chave == "web-02").then(|| "A conexão caiu: tempo esgotado".to_owned())
                    })
                    .on_invoke(move |chave| status.set(format!("Abrindo {chave}")))
                    .on_rename(move |chave, nome| {
                        itens.update(|v| {
                            v.rename(&chave, nome);
                        });
                        renomeando.set(None);
                        status.set("Renomeado".to_owned());
                    })
                    .on_delete(apagar)
                    .on_grid_drop("exemplo", move |soltou| {
                        let Some(pasta) = soltou.target else {
                            return;
                        };
                        soltos.update(|v| v.retain(|l| !soltou.keys.contains(&l.key)));
                        aberto.update(|set| {
                            set.insert(pasta.clone());
                        });
                        status.set(format!("{} em {pasta}", soltou.keys.join(", ")));
                    })
                    .on_move(move |chave, pai, posicao| {
                        let mut moveu = false;
                        itens.update(|v| moveu = v.move_to(&chave, pai.as_deref(), posicao));
                        if moveu {
                            if let Some(pai) = pai {
                                aberto.update(|set| {
                                    set.insert(pai);
                                });
                            }
                            status.set("Movido".to_owned());
                        }
                    }),
            )
            .child(
                DataGrid::bind(soltos, soltos_escolhidos)
                    .id("soltos")
                    .height(112)
                    .columns(vec![DataGridColumn::new("nome", "Arquivos soltos")])
                    .drag_group("exemplo"),
            )
            .child(
                Row::new()
                    .gap(8)
                    .child(
                        Button::new("Pasta")
                            .id("nova-pasta")
                            .height(44)
                            .on_click(move || novo(true)),
                    )
                    .child(
                        Button::new("Novo")
                            .id("novo")
                            .height(44)
                            .on_click(move || novo(false)),
                    )
                    .child(
                        Button::new("Subir")
                            .id("subir")
                            .height(44)
                            .disabled(move || selecionado.get().is_none())
                            .on_click(move || {
                                if let Some(chave) = selecionado.get_untracked() {
                                    itens.update(|v| {
                                        v.move_up(&chave);
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("Descer")
                            .id("descer")
                            .height(44)
                            .disabled(move || selecionado.get().is_none())
                            .on_click(move || {
                                if let Some(chave) = selecionado.get_untracked() {
                                    itens.update(|v| {
                                        v.move_down(&chave);
                                    });
                                }
                            }),
                    ),
            )
            .child(
                Row::new()
                    .gap(8)
                    .child(
                        Button::new("Apagar")
                            .id("apagar")
                            .height(44)
                            .disabled(move || selecionado.get().is_none())
                            .on_click(move || {
                                if let Some(chave) = selecionado.get_untracked() {
                                    apagar(chave);
                                }
                            }),
                    )
                    .child(Text::new(move || status.get()).id("status")),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(servidores).size(420, 640).run()
}
