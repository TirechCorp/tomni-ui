//! `DockHost`: painéis encaixáveis com alfinete. `cargo run --example dock`.
//!
//! Cada `DockPane` tem uma chave, um ícone, um título e um construtor de conteúdo. O estado
//! (modo, lado, tamanhos) mora num `Signal<DockLayout>` que o app guarda e devolve. Um painel
//! **fixado** ocupa espaço ao lado do conteúdo; o alfinete o solta para um botão na faixa da
//! borda, e parar o ponteiro nesse botão (ou clicar) abre o painel por cima do conteúdo; sair
//! dele ou o Esc o recolhe. O ícone de janela do cabeçalho o põe numa janela própria, o duplo
//! clique no título o maximiza e o botão direito abre o menu do cabeçalho.
//!
//! O `DockController` liga menus e atalhos: `controller.toggle("arvore")`, `show`, `close`,
//! `float`, `maximize`... e `controller.shown("arvore")` é um `Signal<bool>` para o
//! `MenuFlyout::toggle`. O conteúdo de um painel é montado de novo quando ele troca de lugar,
//! por isso o estado dele fica em signals de fora. Os ids são `{id}-{chave}` (o painel),
//! `{id}-{chave}-alfinete`, `{id}-{chave}-faixa` e `{id}-conteudo`.

use tomni_ui::prelude::*;

/// A árvore e as propriedades fixadas à esquerda, a ajuda à direita e a saída embaixo, as duas
/// últimas no auto-ocultar. O menu "Exibir" mostra ou esconde cada painel.
pub fn editor() -> impl View {
    let layout = signal(DockLayout::default());
    let controller = DockController::new();
    let arvore = controller.shown("arvore");
    let propriedades = controller.shown("propriedades");
    let ajuda = controller.shown("ajuda");
    let saida = controller.shown("saida");
    let selecionado = signal("web-01".to_owned());

    let conteudo = Column::new()
        .padding(16)
        .gap(8)
        .child(Text::new(move || format!("Selecionado: {}", selecionado.get())).id("estado"));
    let host = DockHost::new(conteudo)
        .id("dock")
        .layout(layout)
        .controller(&controller)
        .min_content(240.0)
        .pane(
            DockPane::new("arvore", Symbol::Folder, "Árvore", move || {
                let mut lista = Column::new().padding(12).gap(4);
                for nome in ["web-01", "web-02", "db-01"] {
                    lista = lista.child(
                        Button::new(nome)
                            .id(format!("no-{nome}"))
                            .on_click(move || selecionado.set(nome.to_owned())),
                    );
                }
                lista
            })
            .size(240.0),
        )
        .pane(
            DockPane::new(
                "propriedades",
                Symbol::Settings,
                "Propriedades",
                move || {
                    Column::new()
                        .padding(12)
                        .child(Text::new(move || format!("Nome: {}", selecionado.get())))
                },
            )
            .size(240.0),
        )
        .pane(
            DockPane::new("ajuda", Symbol::Info, "Ajuda", || {
                Column::new()
                    .padding(12)
                    .child(Text::new("Solte o alfinete para recolher um painel."))
            })
            .side(DockSide::Right)
            .mode(DockMode::AutoHide)
            .size(260.0),
        )
        .pane(
            DockPane::new("saida", Symbol::Document, "Saída", || {
                Column::new()
                    .padding(12)
                    .child(Text::new("10:02 conexão aberta"))
            })
            .side(DockSide::Bottom)
            .mode(DockMode::AutoHide)
            .size(140.0),
        );
    Form::new("Painéis").density(Density::Compact).child(
        Column::new()
            .grow(1)
            .child(
                MenuBar::new().id("barra").menu(
                    "Exibir",
                    MenuFlyout::new()
                        .id("exibir")
                        .toggle("arvore", "Árvore", arvore)
                        .toggle("propriedades", "Propriedades", propriedades)
                        .toggle("ajuda", "Ajuda", ajuda)
                        .toggle("saida", "Saída", saida),
                ),
            )
            .child(host),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(editor).size(960, 600).run()
}
