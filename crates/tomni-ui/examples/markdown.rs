//! `MarkdownView`: o Markdown desenhado com widgets, ao lado do `TextEditor` que o escreve.
//! `cargo run --example markdown --features markdown`.
//!
//! O editor usa o `markdown::highlight` para colorir o texto, e o `on_key` aplica as edições de
//! `markdown::edit`: Enter continua a lista, Tab e Shift+Tab recuam e desfazem o recuo. A
//! prévia é um `MarkdownView` no mesmo signal: marcar uma tarefa nela troca o `[ ]` do texto
//! (`on_task` com `edit::toggle_task`), e um link clicado vai para a linha de estado.

use tomni_ui::markdown::edit::{self, Change};
use tomni_ui::markdown::{MarkdownView, highlight};
use tomni_ui::prelude::*;
use tomni_ui::{Key, TextEditor, TextEditorController};

/// O texto que a tela abre, com um exemplo de cada elemento.
pub const TEXTO: &str = "\
# Notas do servidor

Um parágrafo com **negrito**, *itálico*, ~~riscado~~, `código` e um [link](https://tirech.com.br).

## Tarefas

- [x] Trocar o certificado
- [ ] Conferir o backup
- [ ] Revisar os logs

### Lista

- Primeiro item
  - Subitem
    - Mais um nível
- Segundo item

1. Entrar
2. Reiniciar o serviço

> Citação: reiniciar só depois do backup.

---

```sh
sudo systemctl restart nginx
```

| Servidor | Porta | Estado |
|:---------|:-----:|-------:|
| web-01 | 22 | ativo |
| db-01 | 2222 | parado |

![Diagrama](https://exemplo.com/diagrama.png)
";

/// Aplica a troca no editor: o texto muda e a seleção vai para onde a edição pede.
fn aplicar(controle: &TextEditorController, troca: &Change) {
    controle.replace(troca.range.clone(), &troca.insert);
    controle.set_selection(troca.selection.clone());
}

/// O editor, a prévia e a linha de estado.
pub fn notas() -> impl View {
    let texto = signal(String::from(TEXTO));
    let estado = signal(String::from("Nenhum link aberto"));
    let controle = TextEditorController::new();

    let editor = TextEditor::bind(texto)
        .id("editor")
        .placeholder("Escreva em Markdown")
        .highlight(highlight)
        .controller(controle.clone())
        .on_key(|tecla, controle| {
            let texto = controle.value();
            let selecao = controle.selection();
            let troca = match tecla.key {
                Key::Enter if !tecla.modifiers.shift && selecao.is_empty() => {
                    edit::continue_list(&texto, selecao.end)
                }
                Key::Tab => edit::indent(&texto, selecao, tecla.modifiers.shift),
                _ => None,
            };
            match troca {
                Some(troca) => {
                    aplicar(controle, &troca);
                    true
                }
                None => false,
            }
        });

    let previa = MarkdownView::new(move || texto.get())
        .id("previa")
        .on_link(move |destino| estado.set(format!("Link: {destino}")))
        .on_task(move |byte, _| {
            let atual = texto.get_untracked();
            if let Some(troca) = edit::toggle_task(&atual, byte) {
                texto.set(troca.apply(&atual));
            }
        });

    Form::new("Markdown").child(
        Column::new()
            .gap(8)
            .padding(12)
            .child(
                Split::row(
                    editor,
                    ScrollViewer::new()
                        .id("rolagem")
                        .child(Border::new().padding(16).child(previa)),
                )
                .id("divisao")
                .grow(1),
            )
            .child(Text::new(move || estado.get()).caption().id("estado")),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(notas).size(1000, 720).run()
}
