//! `TextEditor`: texto de várias linhas com quebra, rolagem, desfazer, realce e busca.
//! `cargo run --example text_editor`.
//!
//! O editor liga a um `Signal<String>`. As linhas que começam com `# ` saem em negrito pelo
//! `.highlight`. O `TextEditorController` serve para o app mexer no editor: aqui, achar o
//! texto da busca, marcar os resultados (`.marks`) e levar o cursor ao primeiro. O contador
//! de linhas lê o mesmo signal.

use tomni_ui::prelude::*;
use tomni_ui::{Mark, StyleRange, TextEditor, TextEditorController};

const TEXTO: &str = "# Notas do servidor\n\
Reiniciar o nginx depois de trocar o certificado.\n\
\n\
# Pendências\n\
Conferir o backup da madrugada e a rotação dos logs; este parágrafo é comprido de propósito, \
para o editor quebrar a linha na largura do campo em vez de rolar na horizontal.";

/// As linhas que começam com `# ` em negrito.
fn titulos(texto: &str) -> Vec<StyleRange> {
    let mut faixas = Vec::new();
    let mut inicio = 0;
    for linha in texto.split('\n') {
        if linha.starts_with("# ") {
            faixas.push(StyleRange::new(inicio..inicio + linha.len()).bold());
        }
        inicio += linha.len() + 1;
    }
    faixas
}

/// O editor, a contagem de linhas e a busca.
pub fn editor_de_texto() -> impl View {
    let texto = signal(String::from(TEXTO));
    let busca = signal(String::from("nginx"));
    let marcas = signal(Vec::<Mark>::new());
    let controle = TextEditorController::new();

    let achar = {
        let controle = controle.clone();
        move || {
            let achados = controle.find(&busca.get_untracked(), true);
            marcas.set(
                achados
                    .iter()
                    .enumerate()
                    .map(|(i, r)| Mark {
                        range: r.clone(),
                        current: i == 0,
                    })
                    .collect(),
            );
            if let Some(primeiro) = achados.first() {
                controle.set_selection(primeiro.clone());
                controle.scroll_to(primeiro.start);
            }
        }
    };

    Form::new("Editor de texto").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                Row::new()
                    .gap(8)
                    .child(Edit::bind(busca).id("busca").height(44).grow(1))
                    .child(Button::new("Achar").id("achar").height(44).on_click(achar)),
            )
            .child(
                TextEditor::bind(texto)
                    .id("editor")
                    .placeholder("Escreva aqui")
                    .highlight(titulos)
                    .marks(marcas)
                    .controller(controle),
            )
            .child(
                Text::new(move || {
                    let t = texto.get();
                    format!(
                        "{} linhas · {} caracteres",
                        t.split('\n').count(),
                        t.chars().count()
                    )
                })
                .caption()
                .id("contagem"),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(editor_de_texto).size(480, 560).run()
}
