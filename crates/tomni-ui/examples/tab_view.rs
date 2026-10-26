//! `TabView`: abas no alto e o conteúdo da escolhida embaixo. `cargo run --example tab_view`.
//!
//! A aba escolhida fica num `Signal<String>` com a chave dela. ←/→ andam entre as abas e
//! Ctrl+Tab vai à seguinte de qualquer lugar do conteúdo. As abas têm o id `{id}-{chave}` e os
//! painéis, `{id}-{chave}-conteudo`; os das outras abas ficam montados, só escondidos. No
//! `TestApp`, `app.select("perfil", "Privacidade")` troca pelo título e `app.text("perfil")`
//! lê a chave.
//!
//! A tela `notas` (`--view notas`) é compacta, para desktop: as abas vêm de uma lista
//! (`.items`), o X e o clique do meio fecham (`.on_close`, que tira o item da lista) e o "+"
//! abre uma nota nova (`.on_add`). Arrastar uma aba muda a ordem (`.on_reorder`, que move o
//! item na lista); no `TestApp`, `app.drag("notas-1", "notas-3")`. Com mais abas do que cabem,
//! aparecem as setas de rolagem.
//!
//! A tela `atividade` (`--view atividade`) mostra o que as abas dizem sem serem escolhidas:
//! `.tab_progress` põe um anel no lugar do ícone (indeterminado, com valor, pausado ou com
//! erro) e `.tab_attention` acende um ponto de atenção, que pode ficar por cima do anel. As duas
//! closures são reativas; o `text` da aba ganha o estado ("45%", "pede atenção").

use tomni_ui::prelude::*;

/// Três abas; o campo da primeira guarda o texto quando se troca de aba.
pub fn perfil() -> impl View {
    let aba = signal("dados".to_owned());
    let nome = signal("Ana Souza".to_owned());
    let publico = signal(true);
    Form::new("Perfil").child(
        TabView::bind(aba)
            .id("perfil")
            .grow(1)
            .tab(
                "dados",
                "Dados",
                Column::new()
                    .gap(12)
                    .child(Edit::bind(nome).id("nome").height(44)),
            )
            .tab(
                "privacidade",
                "Privacidade",
                Column::new().gap(12).child(
                    ToggleSwitch::bind(publico)
                        .id("publico")
                        .header("Perfil público"),
                ),
            )
            .tab(
                "sobre",
                "Sobre",
                Text::new("Versão 1.0. Feito com Tomni UI.").wrap(),
            ),
    )
}

/// Uma nota aberta: a chave da aba é o `id`.
#[derive(Clone)]
struct Nota {
    id: u32,
    titulo: String,
    texto: Signal<String>,
}

fn nota(id: u32, titulo: &str, texto: &str) -> Nota {
    Nota {
        id,
        titulo: titulo.to_owned(),
        texto: signal(texto.to_owned()),
    }
}

/// Bloco de notas compacto: uma aba por nota, com o X para fechar, o "+" para abrir outra e o
/// arrasto para mudar a ordem.
pub fn notas() -> impl View {
    let lista = signal(vec![
        nota(1, "Compras", "Pão, café e leite."),
        nota(2, "Ideias", "Um gerenciador de conexões SSH."),
        nota(3, "Reunião", "Quinta, às 10h."),
    ]);
    let aberta = signal("1".to_owned());
    Form::new("Notas").density(Density::Compact).child(
        TabView::bind(aberta)
            .id("notas")
            .grow(1)
            .items(
                lista,
                |n| n.id,
                |n| n.titulo.clone(),
                |n| Edit::bind(n.texto).id(format!("texto-{}", n.id)),
            )
            .on_close(move |chave| lista.update(|v| v.retain(|n| n.id.to_string() != chave)))
            .on_reorder(move |de, para| {
                lista.update(|v| {
                    let nota = v.remove(de);
                    v.insert(para, nota);
                });
            })
            .on_add(move || {
                let id = lista.with_untracked(|v| v.iter().map(|n| n.id).max().unwrap_or(0)) + 1;
                lista.update(|v| v.push(nota(id, &format!("Nota {id}"), "")));
                aberta.set(id.to_string());
            }),
    )
}

/// Quatro tarefas: uma sem valor, uma com 45%, uma pausada e uma com erro e atenção. Clicar na
/// aba que pede atenção apaga o ponto.
pub fn atividade() -> impl View {
    let escolhida = signal("compilar".to_owned());
    let atencao = signal(vec!["testes".to_owned(), "deploy".to_owned()]);
    Form::new("Atividade").density(Density::Compact).child(
        TabView::bind(escolhida)
            .id("atividade")
            .grow(1)
            .tab("compilar", "Compilar", Text::new("Sem valor: o anel gira."))
            .tab("testes", "Testes", Text::new("45%, com um aviso."))
            .tab("backup", "Backup", Text::new("Pausado em 60%."))
            .tab("deploy", "Deploy", Text::new("Com erro e um aviso."))
            .tab_progress(|chave| match chave.as_str() {
                "compilar" => Some(TabProgress {
                    value: None,
                    status: ProgressStatus::Normal,
                }),
                "testes" => Some(TabProgress {
                    value: Some(0.45),
                    status: ProgressStatus::Normal,
                }),
                "backup" => Some(TabProgress {
                    value: Some(0.6),
                    status: ProgressStatus::Paused,
                }),
                "deploy" => Some(TabProgress {
                    value: Some(0.9),
                    status: ProgressStatus::Error,
                }),
                _ => None,
            })
            .tab_attention(move |chave| {
                chave != escolhida.get() && atencao.with(|v| v.contains(&chave))
            }),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(perfil)
        .view(notas)
        .view(atividade)
        .size(390, 844)
        .run()
}
