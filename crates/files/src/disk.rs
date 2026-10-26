//! O espaço livre do disco, para o app avisar antes de baixar ou gravar algo grande.

use std::io;
use std::path::Path;

/// Os bytes livres para o usuário no volume de `path`.
///
/// Se `path` ainda não existe (a pasta de um download que vai ser criada), sobe até o primeiro
/// ancestral que existe e mede o volume dele. É uma função solta, sem estado.
///
/// ```
/// let livre = tomni_ui_files::available_space(&std::env::temp_dir()).unwrap();
/// assert!(livre > 0);
/// ```
///
/// # Errors
///
/// Se nenhum ancestral de `path` existe ou o sistema não responde.
pub fn available_space(path: &Path) -> io::Result<u64> {
    let mut atual = path;
    loop {
        if atual.exists() {
            return fs4::available_space(atual);
        }
        match atual.parent() {
            // Um caminho relativo como `pasta/nova` sobe até `""`, que é a pasta atual.
            Some(pai) if pai.as_os_str().is_empty() => return fs4::available_space("."),
            Some(pai) => atual = pai,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("nenhuma pasta de {} existe", path.display()),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pasta_temporaria(nome: &str) -> std::path::PathBuf {
        let pasta = std::env::temp_dir().join(format!("tomni-disco-{nome}-{}", std::process::id()));
        std::fs::create_dir_all(&pasta).unwrap();
        pasta
    }

    #[test]
    fn diretorio_existente_tem_espaco() {
        let pasta = pasta_temporaria("existe");
        assert!(available_space(&pasta).unwrap() > 0);
        std::fs::remove_dir_all(pasta).unwrap();
    }

    #[test]
    fn caminho_que_ainda_nao_existe_sobe_ate_o_ancestral() {
        let pasta = pasta_temporaria("ancestral");
        let futuro = pasta.join("a").join("b").join("c.bin");
        assert!(available_space(&futuro).unwrap() > 0);
        std::fs::remove_dir_all(pasta).unwrap();
    }

    #[test]
    fn caminho_relativo_que_nao_existe_usa_a_pasta_atual() {
        assert!(available_space(Path::new("nao-existe-mesmo/arquivo")).unwrap() > 0);
    }
}
