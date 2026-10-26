//! Marco M0: render CPU + texto com Selawik + harness de snapshot.

use tomni_ui_render::kurbo::{Point, RoundedRect};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{CanvasExt, Paint, Pixmap};
use tomni_ui_test::{Backend, assert_snapshot, render, snapshot};
use tomni_ui_text::{Fonts, TextStyle};

/// Cores provisórias até o tema chegar no M1.
struct Paleta {
    fundo: Color,
    texto: Color,
    accent: Color,
    texto_no_accent: Color,
}

const CLARO: Paleta = Paleta {
    fundo: Color::from_rgb8(0xF3, 0xF3, 0xF3),
    texto: Color::from_rgb8(0x1B, 0x1B, 0x1B),
    accent: Color::from_rgb8(0x00, 0x5F, 0xB8),
    texto_no_accent: Color::WHITE,
};

const ESCURO: Paleta = Paleta {
    fundo: Color::from_rgb8(0x20, 0x20, 0x20),
    texto: Color::WHITE,
    accent: Color::from_rgb8(0x60, 0xCD, 0xFF),
    texto_no_accent: Color::BLACK,
};

fn cena(paleta: &Paleta) -> Pixmap {
    let mut fonts = Fonts::new();
    render(240, 120, paleta.fundo, |canvas| {
        let titulo = fonts.layout("Tomni UI", TextStyle::BODY_STRONG, None);
        titulo.draw(canvas, Point::new(16.0, 12.0), &Paint::Solid(paleta.texto));

        let corpo = fonts.layout("Ação, coração e órgão.", TextStyle::BODY, None);
        corpo.draw(canvas, Point::new(16.0, 36.0), &Paint::Solid(paleta.texto));

        // Botão accent: 32 px de altura e raio de 4 px, com o rótulo centralizado.
        let botao = RoundedRect::new(16.0, 72.0, 136.0, 104.0, 4.0);
        canvas.fill(&botao, paleta.accent);
        let rotulo = fonts.layout("Entrar", TextStyle::BODY, None);
        let origem = Point::new(
            (76.0 - f64::from(rotulo.width()) / 2.0).round(),
            (88.0 - f64::from(rotulo.height()) / 2.0).round(),
        );
        rotulo.draw(canvas, origem, &Paint::Solid(paleta.texto_no_accent));
    })
}

#[test]
fn m0_claro() {
    assert_snapshot!("m0_claro", cena(&CLARO));
}

#[test]
fn m0_escuro() {
    assert_snapshot!("m0_escuro", cena(&ESCURO));
}

/// Na CPU, bit a bit. Na GPU, o `vello` usa atômicos e a ordem entre as invocações varia
/// (no lavapipe, ~1 pixel em 1 de cada 20 execuções): vale a tolerância por canal.
#[test]
fn render_e_deterministico() {
    let a = cena(&CLARO);
    let b = cena(&CLARO);
    let tolerance = Backend::current().tolerance();
    let comparison = snapshot::compare_within(&a, &b, tolerance).unwrap();
    assert_eq!(comparison.differing, 0);
}
