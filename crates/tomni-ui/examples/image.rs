//! `Image`: PNG, JPEG ou SVG, dos bytes do programa (`include_bytes!`) ou de um arquivo.
//! `cargo run --example image`.
//!
//! Sem `.width` e `.height`, a imagem fica no tamanho natural e encolhe na proporção quando
//! falta largura. Com os dois, o `Stretch` diz como ela ocupa o retângulo: `Uniform` (o
//! padrão) cabe inteira, `UniformToFill` cobre e corta, `Fill` estica e `None` fica no tamanho
//! natural. Toda imagem pede `.alt` com o que ela mostra, ou `.decorative()`; sem os dois, o
//! lint dá o erro `missing-alt`. Um arquivo que não abre vira um quadro vazio, e a árvore
//! mostra o motivo em `error`.

use tomni_ui::prelude::*;

static PAISAGEM: &[u8] = include_bytes!("imagens/paisagem.png");
static FOTO: &[u8] = include_bytes!("imagens/paisagem.jpg");
static LOGO: &[u8] = include_bytes!("imagens/logo.svg");

/// Uma imagem de 150×90 com o `Stretch` `stretch` e a legenda embaixo.
fn amostra(id: &str, stretch: Stretch, legenda: &'static str) -> Column {
    Column::new()
        .gap(4)
        .child(
            Image::bytes(PAISAGEM)
                .id(id)
                .width(150)
                .height(90)
                .stretch(stretch)
                .corner_radius(4)
                .alt("Colinas verdes sob o sol"),
        )
        .child(Text::new(legenda).caption())
}

/// O logotipo em SVG, uma foto JPEG com cantos arredondados e a mesma paisagem nos quatro
/// `Stretch`.
pub fn galeria() -> impl View {
    Form::new("Imagens").child(
        ScrollViewer::new().id("rolagem").child(
            Column::new()
                .gap(16)
                .padding(24)
                .child(
                    Row::new()
                        .gap(12)
                        .align(Align::Center)
                        .child(Image::bytes(LOGO).id("logo").alt("Logotipo do Tomni UI"))
                        .child(Text::new("Galeria").title()),
                )
                .child(
                    Image::bytes(FOTO)
                        .id("foto")
                        .height(160)
                        .stretch(Stretch::UniformToFill)
                        .corner_radius(8)
                        .alt("Colinas verdes sob o sol, em JPEG"),
                )
                .child(
                    UniformGrid::new()
                        .min_column_width(150)
                        .gap(12)
                        .child(amostra("uniform", Stretch::Uniform, "Uniform"))
                        .child(amostra(
                            "uniform-to-fill",
                            Stretch::UniformToFill,
                            "UniformToFill",
                        ))
                        .child(amostra("fill", Stretch::Fill, "Fill"))
                        .child(amostra("none", Stretch::None, "None")),
                ),
        ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(galeria).size(390, 844).run()
}
