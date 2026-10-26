//! Guarda da abertura: troca o modo de desenho quando a abertura anterior caiu antes do
//! primeiro quadro.
//!
//! Um driver de vídeo que derruba o processo ao compilar os shaders do `vello` (ou ao criar a
//! superfície) fecha o app sem tela nenhuma, e a próxima abertura cairia igual. Com
//! [`WindowOptions::render_guard`](crate::WindowOptions::render_guard), o arquivo marca a
//! abertura em andamento antes de tocar na GPU e a desmarca no primeiro quadro apresentado
//! (ou na saída normal sem quadro, como o app que abriu só na bandeja). Se a marca ainda está
//! lá na abertura seguinte, o modo sobe um degrau; o que funcionou fica gravado.
//!
//! Os degraus, do mais rápido ao mais simples:
//!
//! 0. o normal: o adaptador padrão, com o `vello` nos shaders da GPU;
//! 1. o adaptador padrão com o quadro desenhado na CPU (o `CpuRaster` do runner): a GPU só
//!    copia os pixels, sem compute shader;
//! 2. no Windows, o DirectX 12 com o quadro da CPU;
//! 3. no Windows, o OpenGL com o quadro da CPU.
//!
//! Fora do Windows não há o 2 nem o 3: o OpenGL do `wgpu` no Linux pede a conexão com o
//! servidor gráfico ao criar a instância, e sem ela não acha adaptador.
//!
//! Depois do último, volta ao normal: um driver novo pode ter resolvido.

use std::fs;
use std::path::PathBuf;

use tomni_ui_render::vello::util::RenderContext;
use tomni_ui_render::vello::wgpu;

/// Backends tentados a partir do degrau 2, na ordem.
#[cfg(windows)]
const ALTERNATIVES: &[wgpu::Backends] = &[wgpu::Backends::DX12, wgpu::Backends::GL];
#[cfg(not(windows))]
const ALTERNATIVES: &[wgpu::Backends] = &[];

/// O último degrau desta plataforma.
fn last_level() -> u8 {
    1 + ALTERNATIVES.len() as u8
}

/// Como desenhar nesta abertura.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mode {
    /// O degrau, de 0 (o normal) a [`last_level`].
    pub(crate) level: u8,
}

impl Mode {
    /// Se o quadro sai do `vello_cpu` mesmo num adaptador de verdade.
    pub(crate) fn cpu_raster(self) -> bool {
        self.level >= 1
    }

    /// O contexto da GPU, só com os backends do degrau.
    pub(crate) fn context(self) -> RenderContext {
        let Some(&backends) = ALTERNATIVES.get(usize::from(self.level).wrapping_sub(2)) else {
            return RenderContext::new();
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        RenderContext {
            instance,
            devices: Vec::new(),
        }
    }
}

/// O arquivo da guarda e o modo escolhido para esta abertura.
pub(crate) struct RenderGuard {
    path: PathBuf,
    mode: Mode,
    /// Já apresentou um quadro (ou a saída já foi registrada): não grava mais.
    done: bool,
}

/// Marca gravada enquanto a abertura não chegou ao primeiro quadro.
const OPENING: &str = "abrindo";
/// Marca de um degrau que funcionou.
const OK: &str = "ok";

impl RenderGuard {
    /// Lê a abertura anterior, escolhe o degrau desta e marca a abertura em andamento.
    pub(crate) fn open(path: PathBuf) -> Self {
        let level = match fs::read_to_string(&path).ok().as_deref().map(parse) {
            Some(Some((level, false))) => level.min(last_level()),
            // Caiu antes do primeiro quadro: o próximo degrau, ou o normal depois do último.
            Some(Some((level, true))) if level < last_level() => level + 1,
            _ => 0,
        };
        let guard = Self {
            path,
            mode: Mode { level },
            done: false,
        };
        if level > 0 {
            eprintln!(
                "tomni-ui: abrindo no modo de desenho {level} ({})",
                guard.path.display()
            );
        }
        guard.write(OPENING);
        guard
    }

    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }

    /// O primeiro quadro chegou à tela: o degrau funciona.
    pub(crate) fn shown(&mut self) {
        if !self.done {
            self.done = true;
            self.write(OK);
        }
    }

    /// O app terminou. Sem erro, a abertura conta como boa mesmo sem quadro (abriu escondido);
    /// com erro antes do primeiro quadro, a marca fica e a próxima abertura sobe o degrau.
    pub(crate) fn finish(&mut self, failed: bool) {
        if !failed {
            self.shown();
        }
        self.done = true;
    }

    fn write(&self, state: &str) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let text = format!("{} {state}\n", self.mode.level);
        if let Err(e) = fs::write(&self.path, text) {
            eprintln!(
                "tomni-ui: a guarda de desenho não gravou {}: {e}",
                self.path.display()
            );
        }
    }

    #[cfg(test)]
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// `"<degrau> <estado>"`: o degrau e se a abertura ficou no meio.
fn parse(text: &str) -> Option<(u8, bool)> {
    let mut words = text.split_whitespace();
    let level = words.next()?.parse().ok()?;
    let opening = match words.next()? {
        OPENING => true,
        OK => false,
        _ => return None,
    };
    Some((level, opening))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Um arquivo da guarda numa pasta temporária só deste teste.
    fn arquivo(nome: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tomni-ui-guarda-{}-{nome}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("guarda").join("desenho.txt")
    }

    fn conteudo(guard: &RenderGuard) -> String {
        fs::read_to_string(guard.path()).unwrap()
    }

    #[test]
    fn primeira_abertura_fica_no_normal_e_marca_o_andamento() {
        let mut guard = RenderGuard::open(arquivo("primeira"));
        assert_eq!(guard.mode(), Mode { level: 0 });
        assert_eq!(conteudo(&guard), "0 abrindo\n");
        guard.shown();
        assert_eq!(conteudo(&guard), "0 ok\n");
    }

    #[test]
    fn queda_antes_do_quadro_sobe_um_degrau_e_o_que_funciona_fica() {
        let path = arquivo("queda");
        drop(RenderGuard::open(path.clone()));
        let mut guard = RenderGuard::open(path.clone());
        assert_eq!(guard.mode().level, 1);
        assert!(guard.mode().cpu_raster());
        guard.shown();
        let guard = RenderGuard::open(path);
        assert_eq!(guard.mode().level, 1, "o degrau que funcionou continua");
    }

    #[test]
    fn depois_do_ultimo_degrau_volta_ao_normal() {
        let path = arquivo("ultimo");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("{} abrindo\n", last_level())).unwrap();
        assert_eq!(RenderGuard::open(path).mode().level, 0);
    }

    #[test]
    fn saida_normal_sem_quadro_conta_como_boa_e_com_erro_nao() {
        let path = arquivo("saida");
        let mut guard = RenderGuard::open(path.clone());
        guard.finish(false);
        assert_eq!(conteudo(&guard), "0 ok\n");

        let mut guard = RenderGuard::open(path.clone());
        guard.finish(true);
        assert_eq!(conteudo(&guard), "0 abrindo\n");
        assert_eq!(RenderGuard::open(path).mode().level, 1);
    }

    #[test]
    fn arquivo_estragado_volta_ao_normal() {
        assert_eq!(parse("lixo"), None);
        assert_eq!(parse("2"), None);
        assert_eq!(parse("1 ok"), Some((1, false)));
        assert_eq!(parse("3 abrindo\n"), Some((3, true)));
        let path = arquivo("estragado");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "9 qualquer").unwrap();
        assert_eq!(RenderGuard::open(path).mode().level, 0);
    }

    #[test]
    fn degrau_alto_demais_fica_no_ultimo() {
        let path = arquivo("alto");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "200 ok").unwrap();
        assert_eq!(RenderGuard::open(path).mode().level, last_level());
    }
}
