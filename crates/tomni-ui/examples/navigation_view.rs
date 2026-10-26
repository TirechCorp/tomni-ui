//! `NavigationView`: painel de navegação à esquerda e a página do item escolhido ao lado.
//! `cargo run --example navigation_view`.
//!
//! O item escolhido fica num `Signal<String>` com a chave dele. O modo vem da largura da
//! janela: o painel aberto a partir de 1008 px, só os ícones de 641 a 1007 e só o ☰ abaixo;
//! nos dois últimos, o ☰ abre o painel por cima do conteúdo. ↑/↓ andam pelos itens, Enter
//! escolhe e ←/→ fecham e abrem os grupos. Os itens têm o id `{id}-{chave}` e as páginas,
//! `{id}-{chave}-conteudo`; as outras páginas ficam montadas, só escondidas. No `TestApp`,
//! `app.select("biblioteca", "Fotos")` escolhe pelo rótulo (abrindo o grupo ou o painel, se
//! preciso) e `app.text("biblioteca")` lê a chave.

use tomni_ui::prelude::*;

/// Uma página com título e um parágrafo.
fn pagina(titulo: &str, texto: &str) -> Column {
    Column::new()
        .padding(Insets::new(28.0, 40.0, 24.0, 40.0))
        .gap(12)
        .child(Text::new(titulo).title())
        .child(Text::new(texto).wrap())
}

/// Biblioteca com itens, um título de seção, um grupo, a busca e as configurações.
pub fn biblioteca() -> impl View {
    let pagina_atual = signal("inicio".to_owned());
    let busca = signal(String::new());
    let nota = signal(String::new());
    let titulos = ["Dom Casmurro", "Iracema", "O Cortiço", "Vidas Secas"];
    Form::new("Biblioteca").child(
        NavigationView::bind(pagina_atual)
            .id("biblioteca")
            .search(
                AutoSuggestBox::bind(busca, move |texto| {
                    let texto = texto.to_lowercase();
                    titulos
                        .iter()
                        .filter(|t| t.to_lowercase().contains(&texto))
                        .map(|t| (*t).to_owned())
                        .collect()
                })
                .id("busca")
                .height(44)
                .placeholder("Buscar na biblioteca"),
            )
            .item(
                "inicio",
                Symbol::Home,
                "Início",
                pagina("Início", "Os livros, fotos e vídeos guardados aqui.").child(
                    Edit::bind(nota)
                        .id("nota")
                        .height(44)
                        .placeholder("Uma nota que fica ao trocar de página"),
                ),
            )
            .header("Coleção")
            .item(
                "livros",
                Symbol::Book,
                "Livros",
                pagina("Livros", "Quatro livros da coleção."),
            )
            .group("midia", Symbol::ImageMultiple, "Mídia", |g| {
                g.item(
                    "fotos",
                    Symbol::Camera,
                    "Fotos",
                    pagina("Fotos", "As fotos das viagens."),
                )
                .item(
                    "videos",
                    Symbol::Video,
                    "Vídeos",
                    pagina("Vídeos", "Os vídeos da família."),
                )
            })
            .footer_item(
                "conta",
                Symbol::Person,
                "Conta",
                pagina("Conta", "Ana Souza, leitora desde 2019."),
            )
            .settings(pagina("Configurações", "Tema, idioma e notificações.")),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(biblioteca).size(1280, 800).run()
}
