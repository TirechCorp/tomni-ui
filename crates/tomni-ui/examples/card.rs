//! `Card`: um `Border` com o estilo dos cards da Gallery (fundo de card, borda de 1 px, raio 8
//! e padding 16). `cargo run --example card`.
//!
//! Com `.on_click`, o card fica clicável: hover, pressionado, foco pelo teclado, Enter e
//! Espaço. O conteúdo é livre, então o card clicável pede um `.label` (o lint avisa com
//! `missing-label`). Pressionado e desabilitado, os textos de dentro mudam de cor sozinhos.

use tomni_ui::prelude::*;

/// Um card clicável com título e descrição de até duas linhas.
fn opcao(id: &str, titulo: &'static str, descricao: &'static str, aberto: Signal<String>) -> Card {
    Card::new()
        .id(id)
        .label(titulo)
        .on_click(move || aberto.set(titulo.to_owned()))
        .child(
            Column::new()
                .gap(4)
                .child(Text::new(titulo).body_strong())
                .child(Text::new(descricao).secondary().wrap().max_lines(2)),
        )
}

/// Página de configurações: três cards clicáveis, um desabilitado e um só de leitura.
pub fn configuracoes() -> impl View {
    let aberto = signal(String::new());
    Form::new("Configurações").child(
        Column::new()
            .gap(8)
            .padding(24)
            .child(Text::new("Configurações").title())
            .child(opcao(
                "tema",
                "Tema",
                "Claro, escuro ou o do sistema.",
                aberto,
            ))
            .child(opcao(
                "notificacoes",
                "Notificações",
                "Avisos de mensagens, lembretes e atualizações do aplicativo.",
                aberto,
            ))
            .child(opcao(
                "privacidade",
                "Privacidade",
                "O que o aplicativo pode ver: localização, câmera e microfone.",
                aberto,
            ))
            .child(
                opcao(
                    "conta",
                    "Conta",
                    "Entre para sincronizar as configurações.",
                    aberto,
                )
                .disabled(true),
            )
            .child(
                Card::new().id("versao").child(
                    Column::new()
                        .gap(4)
                        .child(Text::new("Versão").body_strong())
                        .child(Text::new("1.4.2, atualizada").secondary()),
                ),
            )
            .child(
                Text::new(move || {
                    let aberto = aberto.get();
                    if aberto.is_empty() {
                        "Escolha uma opção".to_owned()
                    } else {
                        format!("Aberto: {aberto}")
                    }
                })
                .id("aberto"),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(configuracoes).size(390, 844).run()
}
