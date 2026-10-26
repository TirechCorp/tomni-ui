//! Aspas para colar um caminho (ou qualquer texto) como um argumento só no shell.

/// O shell que vai ler o texto.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellQuote {
    /// sh, bash, zsh, fish, dash.
    #[default]
    Posix,
    /// PowerShell (Windows PowerShell 5 e pwsh).
    PowerShell,
    /// cmd.exe.
    Cmd,
}

/// `text` pronto para ser colado como **um** argumento no shell `shell`. Sem aspas quando só
/// tem caracteres seguros.
///
/// - [`ShellQuote::Posix`]: `A-Z a-z 0-9 _ . / : @ % + = , -` ficam como estão; o resto vai
///   entre aspas simples, e o `'` vira `'\''`.
/// - [`ShellQuote::PowerShell`]: `A-Z a-z 0-9 _ . / \ : -` ficam como estão (menos o `-` no
///   começo); o resto vai entre aspas simples, e o `'` (e as aspas tipográficas `‘ ’ ‚ ‛`, que o
///   PowerShell trata como `'`) é dobrado.
/// - [`ShellQuote::Cmd`]: `A-Z a-z 0-9 _ . / \ : -` ficam como estão; o resto vai entre aspas
///   duplas, sem escape (o Windows não deixa `"` em nome de arquivo). O `%` não tem escape no
///   `cmd` interativo e fica como está. Uma quebra de linha vira espaço, porque não há como
///   colá-la sem executar.
///
/// ```
/// use tomni_ui_terminal::{ShellQuote, shell_quote};
///
/// assert_eq!(shell_quote("/tmp/a.png", ShellQuote::Posix), "/tmp/a.png");
/// assert_eq!(shell_quote("/tmp/a b.png", ShellQuote::Posix), "'/tmp/a b.png'");
/// assert_eq!(shell_quote("it's", ShellQuote::Posix), r"'it'\''s'");
/// assert_eq!(shell_quote("it's", ShellQuote::PowerShell), "'it''s'");
/// assert_eq!(shell_quote(r"C:\Ana Maria\x", ShellQuote::Cmd), r#""C:\Ana Maria\x""#);
/// ```
pub fn shell_quote(text: &str, shell: ShellQuote) -> String {
    match shell {
        ShellQuote::Posix => {
            let safe = |c: char| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '_' | '.' | '/' | ':' | '@' | '%' | '+' | '=' | ',' | '-')
            };
            if !text.is_empty() && !text.starts_with('~') && text.chars().all(safe) {
                return text.to_owned();
            }
            let mut out = String::with_capacity(text.len() + 2);
            out.push('\'');
            for c in text.chars() {
                if c == '\'' {
                    out.push_str("'\\''");
                } else {
                    out.push(c);
                }
            }
            out.push('\'');
            out
        }
        ShellQuote::PowerShell => {
            let safe = |c: char| {
                c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '\\' | ':' | '-')
            };
            if !text.is_empty() && !text.starts_with('-') && text.chars().all(safe) {
                return text.to_owned();
            }
            let mut out = String::with_capacity(text.len() + 2);
            out.push('\'');
            for c in text.chars() {
                out.push(c);
                if matches!(c, '\'' | '‘' | '’' | '‚' | '‛') {
                    out.push(c);
                }
            }
            out.push('\'');
            out
        }
        ShellQuote::Cmd => {
            let safe = |c: char| {
                c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '\\' | ':' | '-')
            };
            if !text.is_empty() && text.chars().all(safe) {
                return text.to_owned();
            }
            let mut out = String::with_capacity(text.len() + 2);
            out.push('"');
            out.extend(
                text.chars()
                    .map(|c| if matches!(c, '\r' | '\n') { ' ' } else { c }),
            );
            out.push('"');
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str, shell: ShellQuote) -> String {
        shell_quote(text, shell)
    }

    #[test]
    fn caminho_simples_fica_sem_aspas_nos_tres_shells() {
        for shell in [ShellQuote::Posix, ShellQuote::PowerShell, ShellQuote::Cmd] {
            assert_eq!(q("/tmp/a.png", shell), "/tmp/a.png", "{shell:?}");
        }
    }

    #[test]
    fn espaco_e_acento_pedem_aspas() {
        assert_eq!(
            q("/tmp/relatório final.pdf", ShellQuote::Posix),
            "'/tmp/relatório final.pdf'"
        );
        assert_eq!(
            q("/tmp/relatório final.pdf", ShellQuote::PowerShell),
            "'/tmp/relatório final.pdf'"
        );
        assert_eq!(
            q("/tmp/relatório final.pdf", ShellQuote::Cmd),
            "\"/tmp/relatório final.pdf\""
        );
    }

    #[test]
    fn apostrofo_e_escapado_em_cada_shell() {
        assert_eq!(q("it's.txt", ShellQuote::Posix), r"'it'\''s.txt'");
        assert_eq!(q("it's.txt", ShellQuote::PowerShell), "'it''s.txt'");
    }

    #[test]
    fn til_no_comeco_pede_aspas_no_posix() {
        assert_eq!(q("~/x", ShellQuote::Posix), "'~/x'");
    }

    #[test]
    fn traco_no_comeco_pede_aspas_so_no_powershell() {
        assert_eq!(q("-arquivo", ShellQuote::PowerShell), "'-arquivo'");
        assert_eq!(q("-arquivo", ShellQuote::Posix), "-arquivo");
        assert_eq!(q("-arquivo", ShellQuote::Cmd), "-arquivo");
    }

    #[test]
    fn caminho_do_windows_com_espaco_no_cmd() {
        assert_eq!(
            q(r"C:\Users\Ana Maria\x.png", ShellQuote::Cmd),
            r#""C:\Users\Ana Maria\x.png""#
        );
        assert_eq!(
            q(r"C:\Users\Ana\x.png", ShellQuote::PowerShell),
            r"C:\Users\Ana\x.png"
        );
    }

    #[test]
    fn cifrao_pede_aspas_nos_tres_shells() {
        assert_eq!(q("$HOME.txt", ShellQuote::Posix), "'$HOME.txt'");
        assert_eq!(q("$HOME.txt", ShellQuote::PowerShell), "'$HOME.txt'");
        assert_eq!(q("$HOME.txt", ShellQuote::Cmd), "\"$HOME.txt\"");
    }

    #[test]
    fn aspas_tipograficas_sao_dobradas_no_powershell() {
        assert_eq!(
            q("‘citação’.txt", ShellQuote::PowerShell),
            "'‘‘citação’’.txt'"
        );
        assert_eq!(q("a‚b‛c", ShellQuote::PowerShell), "'a‚‚b‛‛c'");
        // No POSIX elas são texto comum, só pedem aspas por causa do acento.
        assert_eq!(q("‘a’", ShellQuote::Posix), "'‘a’'");
    }

    #[test]
    fn quebra_de_linha_fica_dentro_das_aspas_ou_vira_espaco() {
        assert_eq!(q("a\nb", ShellQuote::Posix), "'a\nb'");
        assert_eq!(q("a\nb", ShellQuote::PowerShell), "'a\nb'");
        assert_eq!(q("a\r\nb", ShellQuote::Cmd), "\"a  b\"");
    }

    #[test]
    fn texto_vazio_vira_aspas_vazias() {
        assert_eq!(q("", ShellQuote::Posix), "''");
        assert_eq!(q("", ShellQuote::PowerShell), "''");
        assert_eq!(q("", ShellQuote::Cmd), "\"\"");
    }

    #[test]
    fn os_caracteres_seguros_do_posix_passam() {
        assert_eq!(
            q("a_b.c/d:e@f%g+h=i,j-k", ShellQuote::Posix),
            "a_b.c/d:e@f%g+h=i,j-k"
        );
    }
}
