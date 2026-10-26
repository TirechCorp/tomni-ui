//! `TitleBar`: a barra de título desenhada pelo app, com o fundo Mica.
//! `cargo run --example title_bar`.
//!
//! A barra vai no `Form::title_bar`, e a janela perde a moldura do sistema. Ela tem o voltar,
//! o ☰ do `NavigationView` (que vai para a barra sozinho), o ícone, o título, o subtítulo, a
//! busca no meio e os botões da janela. Arrastar a barra move a janela, o duplo clique
//! maximiza e as bordas redimensionam. Com a janela inativa, a barra fica apagada. No
//! `TestApp`, `app.window_requests()` lê o que a barra pediu à janela e
//! `app.set_window_active(false)` simula outro app em primeiro plano.

use tomni_ui::prelude::*;

/// Uma página com título e um parágrafo.
fn pagina(titulo: &str, texto: &str) -> Column {
    Column::new()
        .padding(Insets::new(28.0, 40.0, 24.0, 40.0))
        .gap(12)
        .child(Text::new(titulo).title())
        .child(Text::new(texto).wrap())
}

/// A Biblioteca com a barra de título própria: o voltar leva ao início, e a busca fica no
/// meio da barra.
pub fn biblioteca() -> impl View {
    let pagina_atual = signal("inicio".to_owned());
    let busca = signal(String::new());
    let titulos = ["Dom Casmurro", "Iracema", "O Cortiço", "Vidas Secas"];
    Form::new("Biblioteca")
        .title_bar(
            TitleBar::new("Biblioteca")
                .subtitle("Prévia")
                .icon(Symbol::Library)
                .back(true)
                .back_enabled(move || pagina_atual.get() != "inicio")
                .on_back(move || pagina_atual.set("inicio".to_owned()))
                .content(
                    AutoSuggestBox::bind(busca, move |texto| {
                        let texto = texto.to_lowercase();
                        titulos
                            .iter()
                            .filter(|t| t.to_lowercase().contains(&texto))
                            .map(|t| (*t).to_owned())
                            .collect()
                    })
                    .id("busca")
                    .width(360)
                    .placeholder("Buscar na biblioteca"),
                ),
        )
        .child(
            NavigationView::bind(pagina_atual)
                .id("navegacao")
                .item(
                    "inicio",
                    Symbol::Home,
                    "Início",
                    pagina("Início", "Os livros, fotos e vídeos guardados aqui."),
                )
                .header("Coleção")
                .item(
                    "livros",
                    Symbol::Book,
                    "Livros",
                    pagina("Livros", "Quatro livros da coleção."),
                )
                .item(
                    "fotos",
                    Symbol::Camera,
                    "Fotos",
                    pagina("Fotos", "As fotos das viagens."),
                )
                .settings(pagina("Configurações", "Tema, idioma e notificações.")),
        )
}

/// Uma janela pequena com a barra compacta, de 32 px: só o título e os botões da janela.
pub fn notas() -> impl View {
    let nota = signal(String::new());
    Form::new("Notas")
        .title_bar(TitleBar::new("Notas").icon(Symbol::Edit))
        .child(
            Column::new()
                .padding(Insets::all(24.0))
                .gap(12)
                .child(Text::new("Uma nota rápida").subtitle())
                .child(
                    Edit::bind(nota)
                        .id("nota")
                        .height(44)
                        .placeholder("Escreva aqui"),
                ),
        )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(biblioteca)
        .size(1280, 800)
        .backdrop(Backdrop::Mica)
        .run()
}
