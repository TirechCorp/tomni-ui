use fluent_rs::prelude::*;

const EMAIL_CADASTRADO: &str = "ana@exemplo.com";

#[derive(Clone, Copy, PartialEq)]
enum Resultado {
    Nenhum,
    EmailCadastrado,
    ContaCriada,
}

fn cadastro() -> impl View {
    let nome = signal(String::new());
    let email = signal(String::new());
    let senha = signal(String::new());
    let aceito = signal(false);
    let novidades = signal(false);
    let resultado = signal(Resultado::Nenhum);

    Form::new("Criar conta").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Text::new("Criar conta").title())
            .child(
                InfoBar::new(InfoBarSeverity::Error)
                    .id("erro")
                    .title("Erro")
                    .message("Este e-mail já está cadastrado.")
                    .visible(move || resultado.get() == Resultado::EmailCadastrado)
                    .on_close(move || resultado.set(Resultado::Nenhum)),
            )
            .child(
                InfoBar::new(InfoBarSeverity::Success)
                    .id("sucesso")
                    .title("Pronto")
                    .message("Sua conta foi criada.")
                    .visible(move || resultado.get() == Resultado::ContaCriada)
                    .on_close(move || resultado.set(Resultado::Nenhum)),
            )
            .child(Edit::bind(nome).id("nome").placeholder("Nome").height(44))
            .child(
                Edit::bind(email)
                    .id("email")
                    .placeholder("E-mail")
                    .height(44),
            )
            .child(
                Edit::bind(senha)
                    .id("senha")
                    .placeholder("Senha")
                    .password()
                    .height(44),
            )
            .child(
                CheckBox::bind(aceito, "Li e aceito os termos")
                    .id("termos")
                    .height(44),
            )
            .child(
                ToggleSwitch::bind(novidades)
                    .id("novidades")
                    .header("Receber novidades por e-mail"),
            )
            .child(
                Button::new("Criar conta")
                    .id("criar")
                    .accent()
                    .height(44)
                    .disabled(move || !aceito.get())
                    .on_click(move || {
                        let cadastrado = email.get().trim().eq_ignore_ascii_case(EMAIL_CADASTRADO);
                        resultado.set(if cadastrado {
                            Resultado::EmailCadastrado
                        } else {
                            Resultado::ContaCriada
                        });
                    }),
            ),
    )
}

fn main() -> Result<(), fluent_rs::platform::Error> {
    fluent_rs::App::new(cadastro).size(390, 844).run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluent_rs_test::TestApp;

    fn preencher(app: &mut TestApp, email: &str) {
        app.type_into("nome", "Ana Souza")
            .type_into("email", email)
            .type_into("senha", "segredo123");
    }

    #[test]
    fn estado_inicial() {
        let mut app = TestApp::new(cadastro).size(390, 844);
        assert_eq!(app.text("nome"), "");
        assert_eq!(app.text("email"), "");
        assert_eq!(app.text("senha"), "");
        assert!(!app.checked("termos"));
        assert_eq!(app.label("termos"), "Li e aceito os termos");
        assert!(!app.checked("novidades"));
        assert_eq!(app.label("novidades"), "Receber novidades por e-mail");
        assert_eq!(app.label("criar"), "Criar conta");
        assert!(!app.enabled("criar"));
        assert!(!app.visible("erro"));
        assert!(!app.visible("sucesso"));
        app.assert_lint();
    }

    #[test]
    fn botao_depende_dos_termos() {
        let mut app = TestApp::new(cadastro).size(390, 844);
        preencher(&mut app, "bruno@exemplo.com");
        assert!(!app.enabled("criar"));
        app.click("termos");
        assert!(app.checked("termos"));
        assert!(app.enabled("criar"));
        app.click("termos");
        assert!(!app.enabled("criar"));
    }

    #[test]
    fn chave_de_novidades_alterna() {
        let mut app = TestApp::new(cadastro).size(390, 844);
        app.click("novidades");
        assert!(app.checked("novidades"));
        app.click("novidades");
        assert!(!app.checked("novidades"));
    }

    #[test]
    fn email_ja_cadastrado_mostra_erro() {
        let mut app = TestApp::new(cadastro).size(390, 844);
        preencher(&mut app, "ana@exemplo.com");
        app.click("termos");
        app.click("criar");
        assert!(app.visible("erro"));
        assert!(!app.visible("sucesso"));
        assert_eq!(app.text("erro"), "Este e-mail já está cadastrado.");
        app.assert_lint();

        app.click("erro");
        assert!(!app.visible("erro"));
    }

    #[test]
    fn outro_email_mostra_sucesso() {
        let mut app = TestApp::new(cadastro).size(390, 844);
        preencher(&mut app, "bruno@exemplo.com");
        app.click("termos");
        app.click("criar");
        assert!(app.visible("sucesso"));
        assert!(!app.visible("erro"));
        assert_eq!(app.text("sucesso"), "Sua conta foi criada.");
        app.assert_lint();
    }

    #[test]
    fn trocar_email_troca_a_barra() {
        let mut app = TestApp::new(cadastro).size(390, 844);
        preencher(&mut app, "ana@exemplo.com");
        app.click("termos");
        app.click("criar");
        assert!(app.visible("erro"));

        app.type_into("email", ".br");
        app.click("criar");
        assert!(!app.visible("erro"));
        assert!(app.visible("sucesso"));
    }
}
