//! `NumberBox`: campo de número ligado a um `Signal<f64>` (ou `Signal<Option<f64>>` com
//! `bind_optional`, em que vazio vira `None`). `cargo run --example number_box`.
//!
//! Segue o `NumberBox` do WinUI 3. Aceita vírgula ou ponto e mostra com vírgula (`1,5`). O
//! valor muda ao confirmar (Enter ou ao sair do campo), com ↑/↓ (`.small_change`, padrão 1),
//! PageUp/PageDown (`.large_change`, padrão 10) e a roda do mouse com o foco, preso entre
//! `.min` e `.max` e, com `.precision(casas)`, arredondado. Sem `.precision`, o valor fica
//! como veio e o campo mostra até 10 dígitos significativos. Ao receber o foco, o texto fica
//! todo selecionado; Esc volta ao valor; um texto que não é número volta ao valor anterior.
//!
//! Os botões de incremento (`{id}-aumentar`, `{id}-diminuir`) não aparecem por padrão
//! (`SpinButtons::Hidden`, como o WinUI); com `Compact`, ficam num popup sobre o fim do campo
//! enquanto ele tem o foco, e com `Inline`, dentro do campo. Segurados, repetem; no limite,
//! desabilitam. O X (`{id}-limpar`) apaga o texto. No `TestApp`:
//! `app.set_value("quantidade", 3.0)`, `app.value("quantidade")` e
//! `app.press("preco-aumentar")`.

use tomni_ui::prelude::*;

/// Pedido: quantidade, preço e desconto opcional, com o total embaixo.
pub fn pedido() -> impl View {
    let quantidade = signal(1.0);
    let preco = signal(12.5);
    let desconto = signal(None::<f64>);
    let total = move || {
        let bruto = quantidade.get() * preco.get();
        bruto * (1.0 - desconto.get().unwrap_or(0.0) / 100.0)
    };
    Form::new("Pedido").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Text::new("Quantidade"))
            .child(
                NumberBox::bind(quantidade)
                    .id("quantidade")
                    .min(1.0)
                    .max(99.0)
                    .precision(0)
                    .spin_buttons(SpinButtons::Compact)
                    .height(44),
            )
            .child(Text::new("Preço (R$)"))
            .child(
                NumberBox::bind(preco)
                    .id("preco")
                    .min(0.0)
                    .small_change(0.5)
                    .precision(2)
                    .spin_buttons(SpinButtons::Inline)
                    .height(44),
            )
            .child(Text::new("Desconto (%)"))
            .child(
                NumberBox::bind_optional(desconto)
                    .id("desconto")
                    .placeholder("Sem desconto")
                    .min(0.0)
                    .max(100.0)
                    .height(44),
            )
            .child(
                Text::new(move || format!("Total: R$ {}", tomni_ui::format_number(total(), 2)))
                    .id("total")
                    .subtitle(),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(pedido).size(390, 844).run()
}
