//! `For`: uma view para cada item de uma lista, identificada por uma chave.
//! `cargo run --example lista`.
//!
//! Quando a lista muda, só os itens novos ganham view e só os que saíram são descartados: os
//! outros mantêm o estado, mesmo trocando de posição. A view de cada item é criada uma vez,
//! então o que muda num item (a estrela, aqui) fica num signal dentro dele.

use tomni_ui::prelude::*;

/// Uma música da lista.
#[derive(Clone)]
pub struct Musica {
    /// Chave do `For`: nunca se repete.
    pub id: u32,
    /// Nome.
    pub nome: String,
    /// Marcada como favorita. Um signal: muda sem recriar a view do item.
    pub favorita: Signal<bool>,
}

/// Playlist: marcar como favorita, subir uma posição e remover; "Nenhuma música" quando a
/// lista esvazia, e "Restaurar" a enche de novo.
pub fn playlist() -> impl View {
    let nomes = [
        "Aquarela",
        "Garota de Ipanema",
        "Trem das Onze",
        "Asa Branca",
        "Carinhoso",
    ];
    let inicial = move || {
        (1..)
            .zip(nomes)
            .map(|(id, nome)| Musica {
                id,
                nome: nome.to_owned(),
                favorita: signal(false),
            })
            .collect::<Vec<_>>()
    };
    let musicas = signal(inicial());
    let favoritas = move || musicas.with(|m| m.iter().filter(|m| m.favorita.get()).count());
    Form::new("Playlist").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Text::new(move || format!("Favoritas: {}", favoritas())).id("total"))
            .child(For::each(
                musicas,
                |m| m.id,
                move |m| {
                    let id = m.id;
                    Row::new()
                        .gap(8)
                        .align(Align::Center)
                        .child(
                            CheckBox::bind(m.favorita, m.nome)
                                .id(format!("favorita-{id}"))
                                .height(44)
                                .grow(1),
                        )
                        .child(
                            Button::new("Subir")
                                .id(format!("subir-{id}"))
                                .height(44)
                                .on_click(move || {
                                    musicas.update(|m| {
                                        if let Some(i) = m.iter().position(|m| m.id == id)
                                            && i > 0
                                        {
                                            m.swap(i - 1, i);
                                        }
                                    });
                                }),
                        )
                        .child(
                            Button::new("Remover")
                                .id(format!("remover-{id}"))
                                .height(44)
                                .on_click(move || musicas.update(|m| m.retain(|m| m.id != id))),
                        )
                },
            ))
            .child(Show::when(
                move || musicas.with(Vec::is_empty),
                move || {
                    Column::new()
                        .gap(12)
                        .child(Text::new("Nenhuma música").id("vazia"))
                        .child(
                            Button::new("Restaurar")
                                .id("restaurar")
                                .height(44)
                                .on_click(move || musicas.set(inicial())),
                        )
                },
            )),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(playlist).size(390, 844).run()
}
