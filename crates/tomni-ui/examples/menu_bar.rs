//! `MenuBar`, atalhos e menu de contexto. `cargo run --example menu_bar`.
//!
//! A barra fica no alto da janela; cada `.menu(título, MenuFlyout)` vira um item com o id
//! `{id}-{título em slug}`. Clicar abre o menu e, com um aberto, passar o ponteiro sobre outro
//! troca de menu; F10 ou Alt levam o foco à barra. Os itens com `.shortcut()` mostram o atalho
//! à direita e valem com o menu fechado. `.accelerator()` dá um atalho a qualquer widget e
//! `.context_flyout()` dá um menu de contexto (botão direito, Shift+F10, tecla Menu ou toque
//! longo). No `TestApp`: `app.shortcut('O')` (Ctrl+O), `app.shortcut(Shortcut::key(Key::F(5)))`
//! e `app.context_menu("app-log")`.

use tomni_ui::prelude::*;

/// Visualizador de logs: barra de menus, a lista com menu de contexto e o "Recarregar" no F5.
pub fn logs() -> impl View {
    let arquivos = signal(vec!["app.log".to_owned(), "erro.log".to_owned()]);
    let quebra = signal(true);
    let status = signal("Pronto".to_owned());
    let avisa = move |texto: &'static str| move || status.set(texto.to_owned());
    Form::new("Logs").child(
        Column::new()
            .grow(1)
            .child(
                MenuBar::new()
                    .id("barra")
                    .menu(
                        "Arquivo",
                        MenuFlyout::new()
                            .id("arquivo")
                            .item("abrir", "Abrir...", move || {
                                arquivos.update(|v| v.push(format!("log-{}.log", v.len() + 1)));
                            })
                            .icon(Symbol::FolderOpen)
                            .shortcut(Shortcut::ctrl('O'))
                            .submenu(
                                "exibir",
                                "Exibir",
                                MenuFlyout::new()
                                    .id("exibir-menu")
                                    .toggle("quebra", "Quebra de linha", quebra)
                                    .shortcut(Shortcut::ctrl_shift('W')),
                            )
                            .separator()
                            .item("sair", "Sair", avisa("Saindo")),
                    )
                    .access_key('A')
                    .menu(
                        "Ajuda",
                        MenuFlyout::new()
                            .id("ajuda")
                            .item("sobre", "Sobre", avisa("Logs 1.0")),
                    )
                    .access_key('J'),
            )
            .child(
                ScrollViewer::new().id("rolagem").grow(1).child(
                    Column::new()
                        .id("lista")
                        .padding(12)
                        .gap(8)
                        .child(For::each(
                            arquivos,
                            |nome| nome.clone(),
                            |nome| Text::new(nome.clone()).id(nome.replace('.', "-")),
                        ))
                        .context_flyout(
                            MenuFlyout::new()
                                .id("lista-menu")
                                .item("copiar", "Copiar o caminho", avisa("Caminho copiado"))
                                .shortcut(Shortcut::ctrl_shift('C'))
                                .separator()
                                .item("limpar", "Limpar a lista", move || arquivos.set(Vec::new()))
                                .icon(Symbol::Delete),
                        ),
                ),
            )
            .child(
                Row::new()
                    .padding(12)
                    .gap(12)
                    .align(Align::Center)
                    .child(
                        Button::new("Recarregar")
                            .id("recarregar")
                            .height(44)
                            .on_click(avisa("Recarregado"))
                            .accelerator(Shortcut::key(Key::F(5)), avisa("Recarregado")),
                    )
                    .child(
                        Text::new(move || {
                            let quebra = if quebra.get() {
                                "com quebra"
                            } else {
                                "sem quebra"
                            };
                            format!("{}, {quebra}", status.get())
                        })
                        .id("status"),
                    ),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(logs).size(800, 500).run()
}
