//! Lista de comandos de desenho, gravada uma vez e reproduzida em qualquer `Canvas`.

use crate::kurbo::{Affine, BezPath, Rect, Stroke, Vec2};
use crate::peniko::{FontData, GradientKind};
use crate::{Canvas, Glyph, GlyphRun, ImagePaint, Paint};

/// Desenho de um widget, gravado para ser reproduzido a cada frame.
///
/// É a base do repaint seletivo: só os widgets que mudaram gravam de novo, e o frame reproduz
/// as gravações de todos, na ordem da árvore. Reproduzir gera os mesmos pixels que desenhar
/// direto.
#[derive(Clone, Debug, Default)]
pub struct Recording {
    size: (u32, u32),
    commands: Vec<Command>,
}

#[derive(Clone, Debug)]
enum Command {
    FillPath(BezPath, Paint),
    FillRect(Rect, Paint),
    StrokePath(BezPath, Stroke, Paint),
    Glyphs {
        font: FontData,
        font_size: f32,
        normalized_coords: Vec<i16>,
        glyphs: Vec<Glyph>,
        skew: f32,
        paint: Paint,
    },
    PushClip(BezPath),
    PopClip,
    PushOpacity(f32),
    PopOpacity,
}

impl Recording {
    /// Gravação vazia para uma superfície de `width` × `height` pixels.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            size: (width, height),
            commands: Vec::new(),
        }
    }

    /// Se nada foi gravado.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Reproduz os comandos gravados em `canvas`.
    pub fn replay(&self, canvas: &mut (impl Canvas + ?Sized)) {
        self.replay_at(canvas, Vec2::ZERO);
    }

    /// Reproduz os comandos gravados em `canvas`, deslocados por `offset`. É como um widget
    /// gravado uma vez é pintado em outro lugar (rolado, ou depois que o layout o moveu) sem
    /// gravar de novo.
    pub fn replay_at(&self, canvas: &mut (impl Canvas + ?Sized), offset: Vec2) {
        if offset != Vec2::ZERO {
            for command in &self.commands {
                command.translate(offset).run(canvas);
            }
            return;
        }
        for command in &self.commands {
            command.run(canvas);
        }
    }
}

impl Command {
    fn run(&self, canvas: &mut (impl Canvas + ?Sized)) {
        match self {
            Command::FillPath(path, paint) => canvas.fill_path(path, paint),
            Command::FillRect(rect, paint) => canvas.fill_rect(*rect, paint),
            Command::StrokePath(path, stroke, paint) => canvas.stroke_path(path, stroke, paint),
            Command::Glyphs {
                font,
                font_size,
                normalized_coords,
                glyphs,
                skew,
                paint,
            } => canvas.draw_glyphs(
                &GlyphRun {
                    font,
                    font_size: *font_size,
                    normalized_coords,
                    glyphs,
                    skew: *skew,
                },
                paint,
            ),
            Command::PushClip(path) => canvas.push_clip(path),
            Command::PopClip => canvas.pop_clip(),
            Command::PushOpacity(alpha) => canvas.push_opacity(*alpha),
            Command::PopOpacity => canvas.pop_opacity(),
        }
    }

    fn translate(&self, offset: Vec2) -> Self {
        let path = |path: &BezPath| Affine::translate(offset) * path.clone();
        match self {
            Command::FillPath(p, paint) => Command::FillPath(path(p), translate(paint, offset)),
            Command::FillRect(rect, paint) => {
                Command::FillRect(*rect + offset, translate(paint, offset))
            }
            Command::StrokePath(p, stroke, paint) => {
                Command::StrokePath(path(p), stroke.clone(), translate(paint, offset))
            }
            Command::Glyphs {
                font,
                font_size,
                normalized_coords,
                glyphs,
                skew,
                paint,
            } => Command::Glyphs {
                font: font.clone(),
                font_size: *font_size,
                normalized_coords: normalized_coords.clone(),
                glyphs: glyphs
                    .iter()
                    .map(|g| Glyph {
                        id: g.id,
                        x: g.x + offset.x as f32,
                        y: g.y + offset.y as f32,
                    })
                    .collect(),
                skew: *skew,
                paint: translate(paint, offset),
            },
            Command::PushClip(p) => Command::PushClip(path(p)),
            Command::PopClip => Command::PopClip,
            Command::PushOpacity(alpha) => Command::PushOpacity(*alpha),
            Command::PopOpacity => Command::PopOpacity,
        }
    }
}

fn translate(paint: &Paint, offset: Vec2) -> Paint {
    match paint {
        Paint::Solid(_) => paint.clone(),
        Paint::Image(image) => Paint::Image(ImagePaint {
            image: image.image.clone(),
            transform: Affine::translate(offset) * image.transform,
        }),
        Paint::Gradient(gradient) => {
            let mut gradient = gradient.clone();
            match &mut gradient.kind {
                GradientKind::Linear(p) => {
                    p.start += offset;
                    p.end += offset;
                }
                GradientKind::Radial(p) => {
                    p.start_center += offset;
                    p.end_center += offset;
                }
                GradientKind::Sweep(p) => p.center += offset,
            }
            Paint::Gradient(gradient)
        }
    }
}

impl Canvas for Recording {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn fill_path(&mut self, path: &BezPath, paint: &Paint) {
        self.commands
            .push(Command::FillPath(path.clone(), paint.clone()));
    }

    fn fill_rect(&mut self, rect: Rect, paint: &Paint) {
        self.commands.push(Command::FillRect(rect, paint.clone()));
    }

    fn stroke_path(&mut self, path: &BezPath, stroke: &Stroke, paint: &Paint) {
        self.commands.push(Command::StrokePath(
            path.clone(),
            stroke.clone(),
            paint.clone(),
        ));
    }

    fn draw_glyphs(&mut self, run: &GlyphRun<'_>, paint: &Paint) {
        self.commands.push(Command::Glyphs {
            font: run.font.clone(),
            font_size: run.font_size,
            normalized_coords: run.normalized_coords.to_vec(),
            glyphs: run.glyphs.to_vec(),
            skew: run.skew,
            paint: paint.clone(),
        });
    }

    fn push_clip(&mut self, path: &BezPath) {
        self.commands.push(Command::PushClip(path.clone()));
    }

    fn pop_clip(&mut self) {
        self.commands.push(Command::PopClip);
    }

    fn push_opacity(&mut self, alpha: f32) {
        self.commands.push(Command::PushOpacity(alpha));
    }

    fn pop_opacity(&mut self) {
        self.commands.push(Command::PopOpacity);
    }
}
