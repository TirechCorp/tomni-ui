//! `set_strings`: os textos próprios da lib (dicas das abas, botões da janela, setas do
//! `NumberBox`…) em outro idioma. `cargo run --example strings`.
//!
//! A lib escreve alguns textos sozinha: a dica do ✕ da aba, o "Voltar" da barra de título, o
//! "Aumentar" do `NumberBox`, o aviso de colar do terminal. Eles saem em português, a menos que
//! o app troque a tabela com [`tomni_ui::set_strings`], partindo de `Strings::default()`. Chame
//! antes de montar a tela: os widgets já montados podem ficar com o texto velho. Esta tela põe
//! os textos em inglês; os dela mesma (`Text::new`, o título das abas) o app traduz como quiser.

use tomni_ui::prelude::*;
use tomni_ui::{SpinButtons, Strings, set_strings};

/// Troca os textos da lib para o inglês.
fn em_ingles() {
    let mut s = Strings::default();
    s.tab_close = "Close tab".into();
    s.tab_add = "New tab".into();
    s.back = "Back".into();
    s.navigation_close = "Close navigation".into();
    s.navigation_open = "Open navigation".into();
    s.minimize = "Minimize".into();
    s.maximize = "Maximize".into();
    s.restore = "Restore".into();
    s.close = "Close".into();
    s.increase = "Increase".into();
    s.decrease = "Decrease".into();
    s.clear = "Clear".into();
    set_strings(s);
}

/// Abas, barra de título e um `NumberBox`, com os textos da lib em inglês.
pub fn ingles() -> impl View {
    em_ingles();
    let aba = signal("one".to_owned());
    let quantidade = signal(1.0);
    Form::new("Strings")
        .title_bar(TitleBar::new("Strings").back(true))
        .child(
            Column::new()
                .padding(16)
                .gap(12)
                .child(
                    TabView::bind(aba)
                        .id("abas")
                        .height(160)
                        .on_add(|| {})
                        .on_close(|_| {})
                        .tab("one", "One", Text::new("First tab"))
                        .tab("two", "Two", Text::new("Second tab")),
                )
                .child(
                    NumberBox::bind(quantidade)
                        .id("quantidade")
                        .min(1.0)
                        .max(9.0)
                        .precision(0)
                        .spin_buttons(SpinButtons::Compact)
                        .height(44),
                ),
        )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(ingles).size(480, 360).run()
}
