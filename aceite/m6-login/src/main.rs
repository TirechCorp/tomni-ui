use fluent_rs::prelude::*;

const EMAIL_CERTO: &str = "ana@exemplo.com";
const SENHA_CERTA: &str = "segredo123";
const SENHA_MINIMA: usize = 8;

fn login() -> impl View {
    let email = signal(String::new());
    let senha = signal(String::new());
    let entrou = signal(false);
    // Senha da última tentativa errada: o aviso vale enquanto a senha não mudar.
    let senha_errada = signal(None::<String>);

    let valido =
        memo(move || email.get().contains('@') && senha.get().chars().count() >= SENHA_MINIMA);

    Form::new("Login").child(
        Column::new()
            .gap(12)
            .padding(24)
            .justify(Justify::Center)
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
                Row::new()
                    .gap(12)
                    .child(
                        Button::new(move || {
                            if entrou.get() {
                                String::from("Bem-vindo, Ana!")
                            } else if senha_errada.get() == Some(senha.get()) {
                                String::from("Senha incorreta")
                            } else {
                                String::from("Entrar")
                            }
                        })
                        .id("entrar")
                        .accent()
                        .height(44)
                        .width(0)
                        .grow(1)
                        .disabled(move || !valido.get())
                        .on_click(move || {
                            if email.get_untracked() == EMAIL_CERTO
                                && senha.get_untracked() == SENHA_CERTA
                            {
                                senha_errada.set(None);
                                entrou.set(true);
                            } else {
                                senha_errada.set(Some(senha.get_untracked()));
                            }
                        }),
                    )
                    .child(
                        Button::new("Criar conta")
                            .id("criar-conta")
                            .height(44)
                            .width(0)
                            .grow(1),
                    ),
            ),
    )
}

fn main() -> Result<(), fluent_rs::platform::Error> {
    fluent_rs::App::new(login).size(390, 844).run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluent_rs::theme::Theme;
    use fluent_rs_test::TestApp;

    #[test]
    fn comeca_com_entrar_desabilitado() {
        let mut app = TestApp::new(login).size(390, 844);
        assert_eq!(app.label("entrar"), "Entrar");
        assert!(!app.enabled("entrar"));
        assert_eq!(app.label("criar-conta"), "Criar conta");
        assert!(app.enabled("criar-conta"));
        app.assert_lint();
    }

    #[test]
    fn entrar_exige_arroba_e_oito_caracteres() {
        let mut app = TestApp::new(login).size(390, 844);

        // Sem "@" no e-mail.
        app.type_into("email", "anaexemplo.com")
            .type_into("senha", "segredo123");
        assert!(!app.enabled("entrar"));

        // Com "@", mas a senha com 7 caracteres.
        let mut app = TestApp::new(login).size(390, 844);
        app.type_into("email", "ana@exemplo.com")
            .type_into("senha", "1234567");
        assert!(!app.enabled("entrar"));

        // O oitavo caractere habilita.
        app.type_into("senha", "8");
        assert_eq!(app.text("senha"), "12345678");
        assert!(app.enabled("entrar"));
    }

    #[test]
    fn credenciais_certas_dao_boas_vindas() {
        let mut app = TestApp::new(login).size(390, 844);
        app.type_into("email", "ana@exemplo.com")
            .type_into("senha", "segredo123");
        assert!(app.enabled("entrar"));
        app.click("entrar");
        assert_eq!(app.label("entrar"), "Bem-vindo, Ana!");
        app.assert_lint();
    }

    #[test]
    fn senha_errada_avisa_ate_mudar_a_senha() {
        let mut app = TestApp::new(login).size(390, 844);
        app.type_into("email", "ana@exemplo.com")
            .type_into("senha", "errada123");
        app.click("entrar");
        assert_eq!(app.label("entrar"), "Senha incorreta");
        app.assert_lint();

        // Mudar só o e-mail não tira o aviso.
        app.type_into("email", "x");
        assert_eq!(app.label("entrar"), "Senha incorreta");

        // Mudar a senha volta para "Entrar".
        app.type_into("senha", "4");
        assert_eq!(app.label("entrar"), "Entrar");
    }

    #[test]
    fn email_errado_tambem_e_recusado() {
        let mut app = TestApp::new(login).size(390, 844);
        app.type_into("email", "bia@exemplo.com")
            .type_into("senha", "segredo123");
        app.click("entrar");
        assert_eq!(app.label("entrar"), "Senha incorreta");
    }

    #[test]
    fn depois_do_erro_acerta_e_entra() {
        let mut app = TestApp::new(login).size(390, 844);
        app.type_into("email", "ana@exemplo.com")
            .type_into("senha", "segredo12");
        app.click("entrar");
        assert_eq!(app.label("entrar"), "Senha incorreta");

        app.type_into("senha", "3");
        assert_eq!(app.text("senha"), "segredo123");
        assert_eq!(app.label("entrar"), "Entrar");
        app.click("entrar");
        assert_eq!(app.label("entrar"), "Bem-vindo, Ana!");
    }

    #[test]
    fn lint_no_tema_escuro_e_tamanho_padrao() {
        let mut app = TestApp::new(login).theme(Theme::Dark);
        app.type_into("email", "ana@exemplo.com")
            .type_into("senha", "segredo123")
            .click("entrar");
        assert_eq!(app.label("entrar"), "Bem-vindo, Ana!");
        app.assert_lint();
    }
}
