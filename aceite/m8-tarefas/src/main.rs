use fluent_rs::prelude::*;

#[derive(Clone)]
struct Tarefa {
    id: u32,
    texto: String,
    feita: Signal<bool>,
}

fn tarefas() -> impl View {
    let lista = signal(Vec::<Tarefa>::new());
    let proximo = signal(1_u32);
    let texto = signal(String::new());

    let adicionar = move || {
        let novo = texto.get_untracked().trim().to_string();
        if novo.is_empty() {
            return;
        }
        let id = proximo.get_untracked();
        proximo.set(id + 1);
        lista.update(|v| {
            v.push(Tarefa {
                id,
                texto: novo,
                feita: signal(false),
            })
        });
        texto.set(String::new());
    };

    let pendentes = move || lista.with(|v| v.iter().filter(|t| !t.feita.get()).count());

    Form::new("Tarefas").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Text::new("Tarefas").title())
            .child(
                Row::new()
                    .gap(8)
                    .child(
                        Edit::bind(texto)
                            .id("nova")
                            .placeholder("Nova tarefa")
                            .height(44)
                            .grow(1),
                    )
                    .child(
                        Button::new("Adicionar")
                            .id("adicionar")
                            .accent()
                            .height(44)
                            .disabled(move || texto.with(|t| t.trim().is_empty()))
                            .on_click(adicionar),
                    ),
            )
            .child(Text::new(move || format!("{} pendentes", pendentes())).id("pendentes"))
            .child(
                Show::when(
                    move || lista.with(Vec::is_empty),
                    || Text::new("Nenhuma tarefa").id("vazia"),
                )
                .otherwise(move || {
                    ScrollViewer::new().id("lista").grow(1).child(
                        Column::new()
                            .gap(8)
                            .padding(Insets::new(0.0, 12.0, 0.0, 0.0))
                            .child(For::each(
                                lista,
                                |t| t.id,
                                move |t| {
                                    let id = t.id;
                                    Row::new()
                                        .gap(8)
                                        .child(
                                            CheckBox::bind(t.feita, t.texto)
                                                .id(format!("tarefa-{id}"))
                                                .height(44)
                                                .grow(1),
                                        )
                                        .child(
                                            Button::new("Remover")
                                                .id(format!("remover-{id}"))
                                                .height(44)
                                                .on_click(move || {
                                                    lista.update(|v| v.retain(|t| t.id != id))
                                                }),
                                        )
                                },
                            )),
                    )
                }),
            ),
    )
}

fn main() -> Result<(), fluent_rs::platform::Error> {
    fluent_rs::App::new(tarefas).size(390, 844).run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluent_rs::theme::Theme;
    use fluent_rs_test::TestApp;

    fn app() -> TestApp {
        TestApp::new(tarefas).size(390, 844)
    }

    #[test]
    fn comeca_vazia_e_com_botao_desabilitado() {
        let mut app = app();
        assert!(app.visible("vazia"));
        assert_eq!(app.text("vazia"), "Nenhuma tarefa");
        assert!(app.ui().find("lista").is_none());
        assert_eq!(app.text("pendentes"), "0 pendentes");
        assert!(!app.enabled("adicionar"));
        app.type_into("nova", "   ");
        assert!(!app.enabled("adicionar"));
        app.assert_lint();
    }

    #[test]
    fn adicionar_poe_no_fim_e_limpa_o_campo() {
        let mut app = app();
        app.type_into("nova", "Comprar pão");
        assert!(app.enabled("adicionar"));
        app.click("adicionar");
        assert_eq!(app.text("nova"), "");
        assert!(!app.enabled("adicionar"));
        assert!(app.ui().find("vazia").is_none());
        assert_eq!(app.label("tarefa-1"), "Comprar pão");
        assert!(!app.checked("tarefa-1"));

        app.type_into("nova", "Lavar louça").click("adicionar");
        assert_eq!(app.label("tarefa-2"), "Lavar louça");
        assert!(app.rect("tarefa-2").y0 > app.rect("tarefa-1").y0);
        assert_eq!(app.text("pendentes"), "2 pendentes");
        app.assert_lint();
    }

    #[test]
    fn marcar_atualiza_o_contador_e_remover_tira_da_lista() {
        let mut app = app();
        app.type_into("nova", "Um").click("adicionar");
        app.type_into("nova", "Dois").click("adicionar");
        app.click("tarefa-1");
        assert!(app.checked("tarefa-1"));
        assert_eq!(app.text("pendentes"), "1 pendentes");
        app.click("tarefa-1");
        assert!(!app.checked("tarefa-1"));
        assert_eq!(app.text("pendentes"), "2 pendentes");

        app.click("tarefa-2").click("remover-1");
        assert!(app.ui().find("tarefa-1").is_none());
        assert!(app.checked("tarefa-2"));
        assert_eq!(app.text("pendentes"), "0 pendentes");

        app.click("remover-2");
        assert!(app.ui().find("tarefa-2").is_none());
        assert_eq!(app.text("vazia"), "Nenhuma tarefa");
        app.assert_lint();
    }

    #[test]
    fn cinquenta_tarefas_rolam_e_a_ultima_pode_ser_marcada() {
        for tema in [Theme::Light, Theme::Dark] {
            let mut app = TestApp::new(tarefas).theme(tema).size(390, 844);
            for i in 1..=50 {
                app.type_into("nova", &format!("Tarefa {i}"))
                    .click("adicionar");
            }
            assert_eq!(app.text("pendentes"), "50 pendentes");
            assert!(!app.in_view("tarefa-50"));
            app.assert_lint();

            app.click("tarefa-50");
            assert!(app.in_view("tarefa-50"));
            assert!(app.checked("tarefa-50"));
            assert!(app.scroll_offset("lista").y > 0.0);
            assert_eq!(app.text("pendentes"), "49 pendentes");
            // o topo da tela continua no lugar
            assert!(app.in_view("nova"));
            assert!(app.enabled("nova"));
            app.assert_lint();
        }
    }
}
