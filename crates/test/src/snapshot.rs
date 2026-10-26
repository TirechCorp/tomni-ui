//! Comparação de imagens com snapshots de referência em PNG.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use tomni_ui_render::Pixmap;
use tomni_ui_render::peniko::color::PremulRgba8;

/// Fração máxima de pixels diferentes aceita (0,5%, critério de sucesso 1 da spec).
pub const MAX_DIFF_RATIO: f64 = 0.005;

/// Diferença por canal tolerada quando a imagem vem da GPU e a referência da CPU.
///
/// Os dois rasterizadores achatam curvas com 0,25 px de tolerância, mas por algoritmos
/// diferentes: nos cantos de bordas finas e nas bordas dos glifos, a cobertura de um pixel
/// varia até ~10%. Acima de 24 níveis (de 255) o pixel conta como diferente, e o limite de
/// [`MAX_DIFF_RATIO`] vale por cima disso. Entre execuções da CPU, a tolerância é zero.
pub const GPU_CHANNEL_TOLERANCE: u8 = 24;

/// Com esta variável de ambiente igual a `1`, os snapshots são gravados como nova referência.
pub const UPDATE_ENV: &str = "TOMNI_UI_UPDATE_SNAPSHOTS";

/// Pasta das referências no lugar de `snapshots/` do crate. Serve para rodar os testes num
/// dispositivo (o emulador Android do CI), onde o caminho do host não existe.
pub const DIR_ENV: &str = "TOMNI_UI_SNAPSHOT_DIR";

/// Resultado da comparação de duas imagens do mesmo tamanho.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Comparison {
    /// Pixels com algum canal diferente além da tolerância.
    pub differing: usize,
    /// Total de pixels.
    pub total: usize,
}

impl Comparison {
    /// Fração de pixels diferentes, de 0 a 1.
    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.differing as f64 / self.total as f64
        }
    }

    /// Se a diferença está dentro de [`MAX_DIFF_RATIO`].
    pub fn passes(&self) -> bool {
        self.ratio() <= MAX_DIFF_RATIO
    }
}

/// Compara pixel a pixel, exigindo igualdade. Retorna `None` se os tamanhos forem diferentes.
pub fn compare(expected: &Pixmap, actual: &Pixmap) -> Option<Comparison> {
    compare_within(expected, actual, 0)
}

/// Compara pixel a pixel: um pixel difere se algum canal variar mais que `tolerance`.
/// Retorna `None` se os tamanhos forem diferentes.
pub fn compare_within(expected: &Pixmap, actual: &Pixmap, tolerance: u8) -> Option<Comparison> {
    if (expected.width(), expected.height()) != (actual.width(), actual.height()) {
        return None;
    }
    let differing = expected
        .data()
        .iter()
        .zip(actual.data())
        .filter(|(e, a)| differs(**e, **a, tolerance))
        .count();
    Some(Comparison {
        differing,
        total: expected.data().len(),
    })
}

fn differs(e: PremulRgba8, a: PremulRgba8, tolerance: u8) -> bool {
    [(e.r, a.r), (e.g, a.g), (e.b, a.b), (e.a, a.a)]
        .into_iter()
        .any(|(e, a)| e.abs_diff(a) > tolerance)
}

/// Imagem com os pixels diferentes (além de `tolerance`) em vermelho sobre a referência
/// esmaecida.
pub fn diff_image(expected: &Pixmap, actual: &Pixmap, tolerance: u8) -> Option<Pixmap> {
    compare(expected, actual)?;
    let mut out = Pixmap::new(expected.width(), expected.height());
    for (i, (e, a)) in expected.data().iter().zip(actual.data()).enumerate() {
        let x = (i % usize::from(expected.width())) as u16;
        let y = (i / usize::from(expected.width())) as u16;
        let pixel = if !differs(*e, *a, tolerance) {
            let fade = |c: u8| (u16::from(c) / 4 + 191) as u8;
            PremulRgba8 {
                r: fade(e.r),
                g: fade(e.g),
                b: fade(e.b),
                a: 255,
            }
        } else {
            PremulRgba8 {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            }
        };
        out.set_pixel(x, y, pixel);
    }
    Some(out)
}

/// Compara `actual` com `<dir>/<name>.png` e entra em pânico se diferir mais que o limite.
/// [`DIR_ENV`], se definida, troca `dir`.
///
/// Prefira a macro [`assert_snapshot!`](crate::assert_snapshot), que usa a pasta
/// `snapshots/` do crate que está sendo testado.
#[track_caller]
pub fn assert_snapshot(dir: impl AsRef<Path>, name: &str, actual: &Pixmap) {
    let update = std::env::var(UPDATE_ENV).is_ok_and(|v| v == "1");
    assert!(
        !update || crate::Backend::current() == crate::Backend::Cpu,
        "snapshot `{name}`: as referências vêm da CPU; aprove sem {}",
        crate::backend::BACKEND_ENV,
    );
    let dir = std::env::var_os(DIR_ENV).map_or_else(|| dir.as_ref().to_owned(), PathBuf::from);
    let tolerance = crate::Backend::current().tolerance();
    if let Err(message) = check(&dir, name, actual, update, tolerance) {
        panic!("{message}");
    }
}

/// Os três arquivos de um snapshot.
struct Paths {
    reference: PathBuf,
    new: PathBuf,
    diff: PathBuf,
}

impl Paths {
    fn new(dir: &Path, name: &str) -> Self {
        Self {
            reference: dir.join(format!("{name}.png")),
            new: dir.join(format!("{name}.snap.new.png")),
            diff: dir.join(format!("{name}.snap.diff.png")),
        }
    }

    fn remove_outputs(&self) {
        // Os arquivos podem não existir; não há nada a fazer nesse caso.
        let _ = fs::remove_file(&self.new);
        let _ = fs::remove_file(&self.diff);
    }
}

pub(crate) fn check(
    dir: &Path,
    name: &str,
    actual: &Pixmap,
    update: bool,
    tolerance: u8,
) -> Result<(), String> {
    let paths = Paths::new(dir, name);
    // Passar a imagem pelo PNG antes de comparar garante que os dois lados sofreram
    // a mesma conversão de alfa pré-multiplicado.
    let png = encode(actual)?;
    let actual = decode(&png)?;

    if update {
        write(&paths.reference, &png)?;
        paths.remove_outputs();
        return Ok(());
    }

    let Ok(reference_png) = fs::read(&paths.reference) else {
        write(&paths.new, &png)?;
        return Err(format!(
            "snapshot `{name}` não tem referência.\n  revise: {}\n  aprove: {UPDATE_ENV}=1 cargo test",
            paths.new.display(),
        ));
    };
    let reference = decode(&reference_png)?;

    let Some(comparison) = compare_within(&reference, &actual, tolerance) else {
        write(&paths.new, &png)?;
        return Err(format!(
            "snapshot `{name}` mudou de tamanho: referência {}x{}, atual {}x{}.\n  nova: {}",
            reference.width(),
            reference.height(),
            actual.width(),
            actual.height(),
            paths.new.display(),
        ));
    };

    if comparison.passes() {
        paths.remove_outputs();
        return Ok(());
    }

    write(&paths.new, &png)?;
    if let Some(diff) = diff_image(&reference, &actual, tolerance) {
        write(&paths.diff, &encode(&diff)?)?;
    }
    Err(format!(
        "snapshot `{name}` difere em {} de {} pixels ({:.3}%, limite {:.1}%, tolerância {tolerance} por canal).\n  nova: {}\n  diff: {}\n  aprove: {UPDATE_ENV}=1 cargo test",
        comparison.differing,
        comparison.total,
        comparison.ratio() * 100.0,
        MAX_DIFF_RATIO * 100.0,
        paths.new.display(),
        paths.diff.display(),
    ))
}

fn encode(pixmap: &Pixmap) -> Result<Vec<u8>, String> {
    pixmap
        .clone()
        .into_png()
        .map_err(|e| format!("falha ao codificar PNG: {e}"))
}

fn decode(png: &[u8]) -> Result<Pixmap, String> {
    Pixmap::from_png(Cursor::new(png)).map_err(|e| format!("falha ao ler PNG: {e}"))
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("falha ao criar {}: {e}", parent.display()))?;
    }
    fs::write(path, bytes).map_err(|e| format!("falha ao gravar {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u16, height: u16, r: u8) -> Pixmap {
        let mut pixmap = Pixmap::new(width, height);
        for y in 0..height {
            for x in 0..width {
                pixmap.set_pixel(
                    x,
                    y,
                    PremulRgba8 {
                        r,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                );
            }
        }
        pixmap
    }

    fn with_changed_pixels(mut pixmap: Pixmap, count: u16) -> Pixmap {
        for x in 0..count {
            pixmap.set_pixel(
                x,
                0,
                PremulRgba8 {
                    r: 0,
                    g: 255,
                    b: 0,
                    a: 255,
                },
            );
        }
        pixmap
    }

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tomni-ui-snap-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn imagens_iguais_nao_diferem() {
        let c = compare(&solid(10, 10, 9), &solid(10, 10, 9)).unwrap();
        assert_eq!(c.differing, 0);
        assert!(c.passes());
    }

    #[test]
    fn limite_de_meio_por_cento() {
        // 200x100 = 20 000 pixels; 0,5% = 100 pixels.
        let base = solid(200, 100, 9);
        let no_limite = compare(&base, &with_changed_pixels(base.clone(), 100)).unwrap();
        let acima = compare(&base, &with_changed_pixels(base.clone(), 101)).unwrap();
        assert!(no_limite.passes());
        assert!(!acima.passes());
    }

    #[test]
    fn tolerancia_por_canal() {
        let base = solid(10, 10, 100);
        let perto = solid(10, 10, 100 + GPU_CHANNEL_TOLERANCE);
        let longe = solid(10, 10, 101 + GPU_CHANNEL_TOLERANCE);
        assert_eq!(compare(&base, &perto).unwrap().differing, 100);
        let c = compare_within(&base, &perto, GPU_CHANNEL_TOLERANCE).unwrap();
        assert_eq!(c.differing, 0);
        let c = compare_within(&base, &longe, GPU_CHANNEL_TOLERANCE).unwrap();
        assert_eq!(c.differing, 100);
    }

    #[test]
    fn tamanhos_diferentes_nao_comparam() {
        assert_eq!(compare(&solid(10, 10, 0), &solid(10, 11, 0)), None);
    }

    #[test]
    fn fluxo_novo_aprovar_e_regressao() {
        let dir = temp_dir("fluxo");
        let base = solid(40, 40, 50);

        let erro = check(&dir, "tela", &base, false, 0).unwrap_err();
        assert!(erro.contains("não tem referência"), "{erro}");
        assert!(dir.join("tela.snap.new.png").exists());

        check(&dir, "tela", &base, true, 0).unwrap();
        assert!(dir.join("tela.png").exists());
        assert!(!dir.join("tela.snap.new.png").exists());
        check(&dir, "tela", &base, false, 0).unwrap();

        let erro = check(&dir, "tela", &with_changed_pixels(base, 40), false, 0).unwrap_err();
        assert!(erro.contains("difere em 40 de 1600 pixels"), "{erro}");
        assert!(dir.join("tela.snap.diff.png").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
