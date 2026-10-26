use fluent_rs::prelude::*;

const UFS: [&str; 5] = ["MG", "RJ", "SP", "BA", "RS"];

fn minha_conta() -> impl View {
    let nome = signal(String::from("Ana Souza"));
    let uf = signal(String::new());
    let saiu = signal(false);
    let confirmar_exclusao = signal(false);
    let excluida = signal(false);

    Form::new("Minha conta").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Text::new("Minha conta").title())
            .child(
                InfoBar::new(InfoBarSeverity::Informational)
                    .id("saiu")
                    .title("Sessão encerrada")
                    .message("Você saiu da conta.")
                    .visible(saiu)
                    .on_close(move || saiu.set(false)),
            )
            .child(
                InfoBar::new(InfoBarSeverity::Success)
                    .id("excluida")
                    .title("Conta excluída")
                    .message("Sua conta foi excluída.")
                    .visible(excluida)
                    .on_close(move || excluida.set(false)),
            )
            .child(Text::new("Nome"))
            .child(Edit::bind(nome).id("nome").placeholder("Nome").height(44))
            .child(Text::new("UF"))
            .child(
                ComboBox::bind(uf, UFS)
                    .id("uf")
                    .placeholder("Escolha a UF")
                    .height(44),
            )
            .child(
                Button::new("Mais opções").id("mais").height(44).flyout(
                    MenuFlyout::new()
                        .id("menu")
                        .item("sair", "Sair", move || saiu.set(true))
                        .separator()
                        .item("excluir", "Excluir conta", move || {
                            confirmar_exclusao.set(true)
                        }),
                ),
            )
            .child(
                ContentDialog::new("Excluir conta?", confirmar_exclusao)
                    .id("excluir-dialogo")
                    .content(
                        Text::new(
                            "Todos os seus dados serão apagados. Essa ação não pode ser desfeita.",
                        )
                        .wrap(),
                    )
                    .primary("Excluir", move || excluida.set(true))
                    .close("Cancelar"),
            ),
    )
}

fn main() -> Result<(), fluent_rs::platform::Error> {
    fluent_rs::App::new(minha_conta).size(390, 844).run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluent_rs::Key;
    use fluent_rs_test::TestApp;

    fn app() -> TestApp {
        TestApp::new(minha_conta).size(390, 844)
    }

    #[test]
    fn estado_inicial() {
        let mut app = app();
        assert_eq!(app.text("nome"), "Ana Souza");
        assert_eq!(app.text("uf"), "");
        assert!(!app.visible("saiu"));
        assert!(!app.visible("excluida"));
        assert!(!app.is_open("menu"));
        assert!(!app.is_open("excluir-dialogo"));
        app.assert_lint();
    }

    #[test]
    fn edita_nome_e_escolhe_uf() {
        let mut app = app();
        app.type_into("nome", " Lima");
        assert_eq!(app.text("nome"), "Ana Souza Lima");

        app.click("uf");
        assert!(app.is_open("uf"));
        for opcao in ["mg", "rj", "sp", "ba", "rs"] {
            assert!(app.visible(&format!("uf-{opcao}")));
        }
        app.assert_lint();
        app.key(Key::Escape);
        assert!(!app.is_open("uf"));

        app.select("uf", "SP");
        assert_eq!(app.text("uf"), "SP");
        assert!(!app.is_open("uf"));
    }

    #[test]
    fn menu_tem_sair_e_excluir() {
        let mut app = app();
        app.click("mais");
        assert!(app.is_open("menu"));
        assert_eq!(app.label("sair"), "Sair");
        assert_eq!(app.label("excluir"), "Excluir conta");
        app.assert_lint();
    }

    #[test]
    fn sair_mostra_mensagem() {
        let mut app = app();
        app.click("mais").click("sair");
        assert!(!app.is_open("menu"));
        assert!(app.visible("saiu"));
        assert_eq!(app.text("saiu"), "Você saiu da conta.");
        assert!(!app.visible("excluida"));
        app.assert_lint();

        app.click("saiu");
        assert!(!app.visible("saiu"));
    }

    #[test]
    fn excluir_pede_confirmacao_e_exclui() {
        let mut app = app();
        app.click("mais").click("excluir");
        assert!(app.is_open("excluir-dialogo"));
        assert!(!app.visible("excluida"));
        assert_eq!(app.label("excluir-dialogo-primary"), "Excluir");
        assert_eq!(app.label("excluir-dialogo-close"), "Cancelar");
        app.assert_lint();

        app.click("excluir-dialogo-primary");
        assert!(!app.is_open("excluir-dialogo"));
        assert!(app.visible("excluida"));
        assert_eq!(app.text("excluida"), "Sua conta foi excluída.");
        app.assert_lint();
    }

    #[test]
    fn cancelar_nao_exclui() {
        let mut app = app();
        app.click("mais").click("excluir");
        app.click("excluir-dialogo-close");
        assert!(!app.is_open("excluir-dialogo"));
        assert!(!app.visible("excluida"));
        assert!(!app.visible("saiu"));
        assert_eq!(app.text("nome"), "Ana Souza");

        // Esc também fecha sem excluir.
        app.click("mais").click("excluir").key(Key::Escape);
        assert!(!app.is_open("excluir-dialogo"));
        assert!(!app.visible("excluida"));
    }

    #[test]
    fn enter_no_dialogo_confirma() {
        let mut app = app();
        app.click("mais").click("excluir").key(Key::Enter);
        assert!(!app.is_open("excluir-dialogo"));
        assert!(app.visible("excluida"));
    }

    #[test]
    fn tema_escuro_passa_no_lint() {
        let mut app = TestApp::new(minha_conta)
            .theme(fluent_rs::theme::Theme::Dark)
            .size(390, 844);
        app.assert_lint();
        app.click("mais").click("excluir");
        app.assert_lint();
        app.click("excluir-dialogo-primary")
            .click("mais")
            .click("sair");
        assert!(app.visible("saiu"));
        assert!(app.visible("excluida"));
        app.assert_lint();
    }
}
