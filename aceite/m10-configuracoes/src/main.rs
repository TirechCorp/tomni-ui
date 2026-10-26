use fluent_rs::prelude::*;

const CIDADES: [&str; 10] = [
    "Belo Horizonte",
    "Belém",
    "Brasília",
    "Curitiba",
    "Fortaleza",
    "Porto Alegre",
    "Recife",
    "Rio de Janeiro",
    "Salvador",
    "São Paulo",
];

/// Cidades que começam com o texto digitado (sem diferenciar maiúsculas).
fn sugerir_cidades(texto: &str) -> Vec<String> {
    let texto = texto.trim().to_lowercase();
    CIDADES
        .into_iter()
        .filter(|c| !texto.is_empty() && c.to_lowercase().starts_with(&texto))
        .map(str::to_owned)
        .collect()
}

fn configuracoes() -> impl View {
    let aba = signal("geral".to_owned());
    let cidade = signal(String::new());
    let som_e_aparencia = signal(false);
    let volume = signal(50.0);
    let tema = signal("Do sistema".to_owned());

    let geral = Column::new()
        .gap(12)
        .padding(Insets::new(16.0, 0.0, 0.0, 0.0))
        .child(
            AutoSuggestBox::bind(cidade, sugerir_cidades)
                .id("cidade")
                .placeholder("Cidade")
                .height(44),
        )
        .child(Text::new("Sincronizando").id("sincronizando-rotulo"))
        .child(ProgressBar::new().id("sincronizando"));

    let avancado = Column::new()
        .gap(12)
        .padding(Insets::new(16.0, 0.0, 0.0, 0.0))
        .child(
            Expander::new("Som e aparência", som_e_aparencia)
                .id("som-e-aparencia")
                .content(
                    Column::new()
                        .gap(12)
                        .child(Slider::bind(volume, 0..=100).id("volume").header("Volume"))
                        .child(
                            Text::new(move || format!("{:.0}%", volume.get()))
                                .id("volume-valor")
                                .caption(),
                        )
                        .child(
                            RadioButtons::bind(tema, ["Claro", "Escuro", "Do sistema"])
                                .id("tema")
                                .header("Tema"),
                        ),
                ),
        );

    Form::new("Configurações").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Text::new("Configurações").title())
            .child(
                TabView::bind(aba)
                    .id("config")
                    .grow(1)
                    .tab("geral", "Geral", geral)
                    .tab("avancado", "Avançado", avancado),
            ),
    )
}

fn main() -> Result<(), fluent_rs::platform::Error> {
    fluent_rs::App::new(configuracoes).size(390, 844).run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluent_rs::theme::Theme;
    use fluent_rs_test::TestApp;

    fn app() -> TestApp {
        TestApp::new(configuracoes).size(390, 844)
    }

    #[test]
    fn comeca_na_aba_geral() {
        let mut app = app();
        assert_eq!(app.text("config"), "geral");
        assert!(app.checked("config-geral"));
        assert!(app.visible("cidade"));
        assert!(app.visible("sincronizando"));
        assert!(!app.visible("som-e-aparencia"));
        assert_eq!(app.text("cidade"), "");
        app.assert_lint();
    }

    #[test]
    fn sugere_cidades_ao_digitar() {
        let mut app = app();
        app.type_into("cidade", "Be");
        assert!(app.is_open("cidade"));
        assert!(app.visible("cidade-belo-horizonte"));
        assert!(app.visible("cidade-belém"));
        assert!(app.ui().find("cidade-brasília").is_none());
        app.assert_lint();

        app.type_text("lé");
        assert!(app.ui().find("cidade-belo-horizonte").is_none());
        app.click("cidade-belém");
        assert!(!app.is_open("cidade"));
        assert_eq!(app.text("cidade"), "Belém");
    }

    #[test]
    fn escolhe_sugestao_pelo_teclado() {
        let mut app = app();
        app.type_into("cidade", "sa");
        assert!(app.is_open("cidade"));
        app.key(fluent_rs::Key::Down).key(fluent_rs::Key::Enter);
        assert!(!app.is_open("cidade"));
        assert_eq!(app.text("cidade"), "Salvador");
    }

    #[test]
    fn sem_sugestoes_a_lista_fica_fechada() {
        let mut app = app();
        app.type_into("cidade", "xyz");
        assert!(!app.is_open("cidade"));
    }

    #[test]
    fn sugerir_cidades_filtra_pelo_comeco() {
        assert!(sugerir_cidades("").is_empty());
        assert_eq!(sugerir_cidades("rio"), vec!["Rio de Janeiro"]);
        assert_eq!(sugerir_cidades("SÃO"), vec!["São Paulo"]);
        assert_eq!(sugerir_cidades("r"), vec!["Recife", "Rio de Janeiro"]);
    }

    #[test]
    fn sincronizando_e_indeterminado() {
        let app = app();
        assert_eq!(app.text("sincronizando-rotulo"), "Sincronizando");
        assert!(app.visible("sincronizando"));
        // `value` falha numa barra sem fim definido.
        let valor =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.value("sincronizando")));
        assert!(valor.is_err(), "a barra devia ser indeterminada");
    }

    #[test]
    fn avancado_tem_som_e_aparencia_fechado() {
        let mut app = app();
        app.select("config", "Avançado");
        assert_eq!(app.text("config"), "avancado");
        assert!(app.visible("som-e-aparencia"));
        assert_eq!(app.label("som-e-aparencia"), "Som e aparência");
        assert!(!app.is_open("som-e-aparencia"));
        assert!(!app.visible("cidade"));
        app.assert_lint();
    }

    #[test]
    fn som_e_aparencia_abre_e_fecha() {
        let mut app = app();
        app.select("config", "Avançado").click("som-e-aparencia");
        assert!(app.is_open("som-e-aparencia"));
        assert_eq!(app.label("volume"), "Volume");
        assert_eq!(app.value("volume"), 50.0);
        assert_eq!(app.text("volume-valor"), "50%");
        assert_eq!(app.text("tema"), "Do sistema");
        assert!(app.checked("tema-do-sistema"));
        assert!(!app.checked("tema-claro"));
        assert!(!app.checked("tema-escuro"));
        app.assert_lint();

        app.click("som-e-aparencia");
        assert!(!app.is_open("som-e-aparencia"));
        assert!(!app.visible("volume"));
    }

    #[test]
    fn muda_volume_e_tema() {
        let mut app = app();
        app.select("config", "Avançado").click("som-e-aparencia");
        app.set_value("volume", 70.0);
        assert_eq!(app.value("volume"), 70.0);
        assert_eq!(app.text("volume-valor"), "70%");
        app.select("tema", "Escuro");
        assert_eq!(app.text("tema"), "Escuro");
        assert!(app.checked("tema-escuro"));
        assert!(!app.checked("tema-do-sistema"));
    }

    #[test]
    fn cada_aba_guarda_o_que_foi_feito() {
        let mut app = app();
        app.type_into("cidade", "Curi");
        app.click("cidade-curitiba");
        assert_eq!(app.text("cidade"), "Curitiba");

        app.select("config", "Avançado").click("som-e-aparencia");
        app.set_value("volume", 20.0).select("tema", "Claro");

        app.select("config", "Geral");
        assert_eq!(app.text("config"), "geral");
        assert_eq!(app.text("cidade"), "Curitiba");

        app.select("config", "Avançado");
        assert!(app.is_open("som-e-aparencia"));
        assert_eq!(app.value("volume"), 20.0);
        assert_eq!(app.text("tema"), "Claro");
        assert!(app.checked("tema-claro"));
    }

    #[test]
    fn lint_no_tema_escuro() {
        let mut app = TestApp::new(configuracoes)
            .size(390, 844)
            .theme(Theme::Dark);
        app.assert_lint();
        app.type_into("cidade", "B");
        assert!(app.is_open("cidade"));
        app.assert_lint();
        app.key(fluent_rs::Key::Escape);
        app.select("config", "Avançado").click("som-e-aparencia");
        app.assert_lint();
    }
}
