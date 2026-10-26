//! `CpuCanvas` com escala: o que a janela usa quando a GPU é um driver em software.

use tomni_ui_render::kurbo::{Affine, BezPath, Circle, Point, Rect, Shape, Stroke};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CpuCanvas, Glyph, GlyphRun, ImagePaint, Paint};
use tomni_ui_text::{Fonts, TextStyle};

const FUNDO: Color = Color::from_rgb8(0xF3, 0xF3, 0xF3);
const AZUL: Color = Color::from_rgb8(0x00, 0x5F, 0xB8);
const TEXTO: Color = Color::from_rgb8(0x1A, 0x1A, 0x1A);

/// Repassa o desenho a um canvas de escala 1 com tudo multiplicado por 2, à mão.
struct Dobro<'a>(&'a mut CpuCanvas);

impl Dobro<'_> {
    fn paint(paint: &Paint) -> Paint {
        match paint {
            Paint::Image(image) => Paint::Image(ImagePaint {
                transform: Affine::scale(2.0) * image.transform,
                ..image.clone()
            }),
            outro => outro.clone(),
        }
    }
}

impl Canvas for Dobro<'_> {
    fn size(&self) -> (u32, u32) {
        let (w, h) = self.0.size();
        (w / 2, h / 2)
    }

    fn fill_path(&mut self, path: &BezPath, paint: &Paint) {
        self.0
            .fill_path(&(Affine::scale(2.0) * path), &Self::paint(paint));
    }

    fn stroke_path(&mut self, path: &BezPath, stroke: &Stroke, paint: &Paint) {
        let stroke = Stroke {
            width: stroke.width * 2.0,
            ..stroke.clone()
        };
        self.0
            .stroke_path(&(Affine::scale(2.0) * path), &stroke, &Self::paint(paint));
    }

    fn draw_glyphs(&mut self, run: &GlyphRun<'_>, paint: &Paint) {
        let glyphs: Vec<Glyph> = run
            .glyphs
            .iter()
            .map(|g| Glyph {
                x: g.x * 2.0,
                y: g.y * 2.0,
                ..*g
            })
            .collect();
        let run = GlyphRun {
            font_size: run.font_size * 2.0,
            glyphs: &glyphs,
            ..*run
        };
        self.0.draw_glyphs(&run, &Self::paint(paint));
    }

    fn push_clip(&mut self, path: &BezPath) {
        self.0.push_clip(&(Affine::scale(2.0) * path));
    }

    fn pop_clip(&mut self) {
        self.0.pop_clip();
    }
}

/// Retângulo, texto, contorno, recorte e imagem, em coordenadas lógicas.
fn desenha(canvas: &mut dyn Canvas, fonts: &mut Fonts) {
    canvas.fill_rect(Rect::new(4.0, 4.0, 36.0, 12.0), &Paint::Solid(AZUL));
    fonts.layout("Ação", TextStyle::BODY, None).draw(
        canvas,
        Point::new(4.0, 14.0),
        &Paint::Solid(TEXTO),
    );
    canvas.stroke_path(
        &Circle::new((32.0, 26.0), 5.0).to_path(0.1),
        &Stroke::new(1.0),
        &Paint::Solid(AZUL),
    );
    canvas.push_clip(&Rect::new(0.0, 32.0, 20.0, 35.0).to_path(0.1));
    let pixels = [
        0xFF, 0, 0, 0xFF, 0, 0xFF, 0, 0xFF, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    let image = tomni_ui_render::ImageData {
        data: pixels.to_vec().into(),
        format: tomni_ui_render::peniko::ImageFormat::Rgba8,
        alpha_type: tomni_ui_render::peniko::ImageAlphaType::Alpha,
        width: 2,
        height: 2,
    };
    let rect = Rect::new(2.0, 30.0, 18.0, 36.0);
    canvas.fill_rect(rect, &Paint::Image(ImagePaint::fit(image, rect)));
    canvas.pop_clip();
}

#[test]
fn escala_2_desenha_como_coordenadas_dobradas() {
    let mut fonts = Fonts::new();
    let mut escalado = CpuCanvas::scaled(80, 72, 2.0);
    assert_eq!(escalado.size(), (40, 36), "size é o tamanho lógico");
    desenha(&mut escalado, &mut fonts);
    let mut dobrado = CpuCanvas::new(80, 72);
    desenha(&mut Dobro(&mut dobrado), &mut fonts);
    let (a, b) = (escalado.render(FUNDO), dobrado.render(FUNDO));
    let tinta = a
        .data_as_u8_slice()
        .chunks(4)
        .filter(|p| p[..3] != [0xF3, 0xF3, 0xF3])
        .count();
    assert!(tinta > 500, "faltou desenho ({tinta} pixels)");
    let diferentes = a
        .data_as_u8_slice()
        .chunks(4)
        .zip(b.data_as_u8_slice().chunks(4))
        .filter(|(p, q)| p.iter().zip(*q).any(|(x, y)| x.abs_diff(*y) > 2))
        .count();
    assert_eq!(
        diferentes, 0,
        "pixels diferentes entre escala 2 e o dobro à mão"
    );
}

#[test]
fn reset_troca_tamanho_e_escala() {
    let mut fonts = Fonts::new();
    let mut canvas = CpuCanvas::scaled(40, 36, 1.0);
    desenha(&mut canvas, &mut fonts);
    canvas.reset(80, 72, 2.0);
    assert_eq!(canvas.size(), (40, 36));
    desenha(&mut canvas, &mut fonts);
    let mut novo = CpuCanvas::scaled(80, 72, 2.0);
    desenha(&mut novo, &mut fonts);
    assert_eq!(
        canvas.render(FUNDO).data_as_u8_slice(),
        novo.render(FUNDO).data_as_u8_slice(),
        "o reset não apagou o frame anterior"
    );
}
