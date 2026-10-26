//! `SettingsCard` e `SettingsExpander`: as linhas de uma página de configurações, como as das
//! Configurações do Windows 11. `cargo run --example settings`.
//!
//! Cada linha tem ícone, cabeçalho, descrição e, à direita, o controle. Abaixo de 476 px de
//! largura **do card** o controle desce para baixo do texto, e abaixo de 286 o ícone some.
//! Com `.on_click` o card inteiro é um botão com o chevron à direita; o
//! `SettingsExpander` abre e fecha uma lista de linhas.

use tomni_ui::prelude::*;

/// Cabeçalho de seção da página (`BodyStrong`, com 30 px de respiro em cima, menos o primeiro).
fn secao(titulo: &'static str, primeira: bool) -> impl View {
    Column::new()
        .padding(Insets::new(
            if primeira { 0.0 } else { 30.0 },
            0.0,
            6.0,
            1.0,
        ))
        .child(Text::new(titulo).body_strong())
}

/// A página: duas seções, um card com `ToggleSwitch`, um com `ComboBox`, um clicável, um
/// desabilitado e um `SettingsExpander` com três filhos.
pub fn tela() -> impl View {
    let notificacoes = signal(true);
    let idioma = signal("Português (Brasil)".to_owned());
    let aberto = signal(false);
    let ligado = signal(true);
    let ultimo = signal(String::new());
    Form::new("Configurações").child(
        ScrollViewer::new().id("pagina").child(
            Column::new()
                .gap(4)
                .padding(Insets::symmetric(24.0, 36.0))
                .child(Text::new("Configurações").title())
                .child(secao("Geral", true))
                .child(
                    SettingsCard::new("Notificações")
                        .id("notificacoes")
                        .description("Avisos de mensagens, lembretes e atualizações.")
                        .icon(Symbol::Speaker)
                        .content(
                            ToggleSwitch::bind(notificacoes)
                                .id("notificacoes-chave")
                                .height(44),
                        ),
                )
                .child(
                    SettingsCard::new("Idioma")
                        .id("idioma")
                        .description("O idioma dos menus e das mensagens.")
                        .icon(Symbol::Globe)
                        .content(
                            ComboBox::bind(idioma, ["Português (Brasil)", "English", "Español"])
                                .id("idioma-lista")
                                .width(200)
                                .height(44),
                        ),
                )
                .child(
                    SettingsCard::new("Sobre")
                        .id("sobre")
                        .description("Versão 1.4.2, atualizada.")
                        .icon(Symbol::Info)
                        .on_click(move || ultimo.set("Sobre".to_owned())),
                )
                .child(
                    SettingsCard::new("Pasta de dados")
                        .id("pasta")
                        .description("Onde ficam as conexões e o cofre.")
                        .icon(Symbol::Folder)
                        .on_click(move || ultimo.set("Pasta".to_owned()))
                        .action_icon(Symbol::Open),
                )
                .child(
                    SettingsCard::new("Conta")
                        .id("conta")
                        .description("Entre para sincronizar as configurações.")
                        .icon(Symbol::Key)
                        .on_click(move || ultimo.set("Conta".to_owned()))
                        .disabled(true),
                )
                .child(secao("Terminal", false))
                .child(
                    SettingsExpander::new("Aparência", aberto)
                        .id("aparencia")
                        .description("Fonte, cores e o cursor.")
                        .icon(Symbol::WindowConsole)
                        .content(ToggleSwitch::bind(ligado).id("aparencia-chave").height(44))
                        .item(
                            SettingsCard::new("Tamanho da fonte")
                                .id("fonte")
                                .content(Text::new("14 px")),
                        )
                        .item(
                            SettingsCard::new("Esquema de cores")
                                .id("esquema")
                                .description("O esquema do terminal, não o do app.")
                                .content(Text::new("Campbell")),
                        )
                        .item(
                            SettingsCard::new("Formato do cursor")
                                .id("cursor")
                                .on_click(move || ultimo.set("Cursor".to_owned())),
                        ),
                )
                .child(
                    Text::new(move || {
                        let ultimo = ultimo.get();
                        if ultimo.is_empty() {
                            "Nada aberto".to_owned()
                        } else {
                            format!("Aberto: {ultimo}")
                        }
                    })
                    .id("ultimo"),
                ),
        ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(tela).size(800, 700).run()
}
