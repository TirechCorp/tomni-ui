//! `Split`: dois painéis com um divisor que o usuário arrasta. `cargo run --example split`.
//!
//! `Split::row` põe os painéis lado a lado e `Split::column`, um em cima do outro. O tamanho do
//! primeiro painel fica num `Signal<f64>`: arrastar o divisor, ou as setas com o foco nele,
//! mudam o signal, e o app pode guardá-lo para a próxima vez. `.min_first` e `.min_second`
//! limitam os painéis. As partes têm os ids `{id}-primeiro`, `{id}-divisor` e `{id}-segundo`;
//! no `TestApp`, `app.value("lados")` lê o tamanho.

use tomni_ui::prelude::*;

/// Lista de servidores à esquerda e, à direita, a sessão em cima e o registro embaixo.
pub fn servidores() -> impl View {
    let lateral = signal(220.0);
    let sessao = signal(300.0);
    let mut lista = Column::new()
        .padding(12)
        .gap(8)
        .child(Text::new("Servidores").body_strong());
    for nome in ["web-01", "web-02", "db-01", "cache-01", "backup-01"] {
        lista = lista.child(Text::new(nome).id(nome));
    }
    Form::new("Servidores").child(
        Split::row(
            ScrollViewer::new().id("lista").child(lista),
            Split::column(
                Column::new()
                    .padding(12)
                    .child(Text::new("Conectado a web-01.").id("sessao-texto")),
                ScrollViewer::new().id("registro").child(
                    Column::new()
                        .padding(12)
                        .gap(4)
                        .child(Text::new("Registro").body_strong())
                        .child(Text::new("10:02 conexão aberta"))
                        .child(Text::new("10:05 comando enviado")),
                ),
            )
            .id("sessao")
            .size(sessao)
            .min_first(120.0)
            .min_second(80.0),
        )
        .id("lados")
        .size(lateral)
        .min_first(160.0)
        .min_second(320.0),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(servidores).size(900, 600).run()
}
