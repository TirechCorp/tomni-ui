//! Os links do terminal (M49): o que está sob um ponto da tela, seja o link que o programa
//! marcou com o `OSC 8` (explícito) ou uma URL `http://` e `https://` no texto (detectada). O
//! endereço local sem esquema (`localhost:3000`, `127.0.0.1:8080`, `0.0.0.0:8000`, `[::1]:5173`)
//! também é detectado e vira `http://…`.
//!
//! As funções daqui são puras: leem o [`Emulator`] e não dependem de janela nem de ponteiro.

use crate::cell::Flags;
use crate::emulator::Emulator;

/// Um link achado sob um ponto da tela.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkHit {
    /// O destino: a URI do `OSC 8`, ou a URL do texto.
    pub uri: String,
    /// Se o programa marcou o link (`OSC 8`); `false` quando foi detectado no texto.
    pub explicit: bool,
    /// As células do link que aparecem na vista, como (linha da vista, coluna). Uma célula larga
    /// entra com as suas duas colunas.
    pub cells: Vec<(usize, usize)>,
}

/// O link sob a célula (`row`, `col`) da vista. O link explícito vale sempre; o detectado só
/// com `detect` ligado (o terminal liga com o Ctrl apertado e no clique direito).
#[must_use]
pub fn find_link(emu: &Emulator, row: usize, col: usize, detect: bool) -> Option<LinkHit> {
    let cell = emu.view_row(row).cells.get(col)?;
    if cell.link.is_some() {
        let link = emu.link(cell)?;
        let mut cells = Vec::new();
        for r in 0..emu.rows() {
            for (c, other) in emu.view_row(r).cells.iter().enumerate() {
                if other.link == cell.link {
                    cells.push((r, c));
                }
            }
        }
        return Some(LinkHit {
            uri: link.uri().to_owned(),
            explicit: true,
            cells,
        });
    }
    if detect {
        detect_url(emu, row, col)
    } else {
        None
    }
}

/// Uma célula da linha lógica, sem as segundas metades dos largos nem o enchimento do fim.
struct Slot {
    line: u64,
    col: usize,
    wide: bool,
}

/// Procura a URL do texto que cobre a célula (`row`, `col`) da vista.
fn detect_url(emu: &Emulator, row: usize, col: usize) -> Option<LinkHit> {
    let clicked = emu.view_line(row);
    let (first, last) = emu.line_bounds();
    // A linha lógica: as linhas ligadas por `wrapped`.
    let mut start = clicked;
    while start > first && emu.line_at(start - 1).is_some_and(|r| r.wrapped) {
        start -= 1;
    }
    let mut end = clicked;
    while end < last && emu.line_at(end).is_some_and(|r| r.wrapped) {
        end += 1;
    }

    let mut slots: Vec<Slot> = Vec::new();
    // Cada caractere e o índice da célula (em `slots`) a que pertence.
    let mut chars: Vec<(char, usize)> = Vec::new();
    for line in start..=end {
        let Some(r) = emu.line_at(line) else { continue };
        for (c, cell) in r.cells.iter().enumerate() {
            if cell.flags.intersects(Flags::WIDE_SPACER | Flags::WIDE_PAD) {
                continue;
            }
            slots.push(Slot {
                line,
                col: c,
                wide: cell.flags.contains(Flags::WIDE),
            });
            let index = slots.len() - 1;
            chars.push((cell.c, index));
            if let Some(combining) = &cell.combining {
                chars.extend(combining.chars().map(|ch| (ch, index)));
            }
        }
    }

    // A célula clicada; o meio de um caractere largo conta como ele.
    let target = slots
        .iter()
        .position(|s| s.line == clicked && (s.col == col || (s.wide && s.col + 1 == col)))?;

    let mut i = 0;
    while i < chars.len() {
        // O endereço local sem esquema só vale depois de espaço, de começo de linha ou de
        // `(["'<`, o que deixa de fora `user@localhost:22` e `xlocalhost:3`.
        let (scheme, local) = match scheme_len(&chars[i..]) {
            0 => (local_len(&chars[i..]), true),
            n => (n, false),
        };
        let at_boundary = if local {
            i == 0 || chars[i - 1].0.is_whitespace() || "([\"'<".contains(chars[i - 1].0)
        } else {
            i == 0 || !chars[i - 1].0.is_alphanumeric()
        };
        if scheme == 0 || !at_boundary {
            i += 1;
            continue;
        }
        let mut j = i + scheme;
        while j < chars.len() && !ends_url(chars[j].0) {
            j += 1;
        }
        let text: Vec<char> = chars[i..j].iter().map(|&(c, _)| c).collect();
        let len = trimmed_len(&text);
        // O endereço local já é o link inteiro; depois do esquema precisa haver mais texto.
        if len >= scheme + usize::from(!local) {
            let (from, to) = (chars[i].1, chars[i + len - 1].1);
            if (from..=to).contains(&target) {
                let origin = emu.view_line(0);
                let mut cells = Vec::new();
                for slot in &slots[from..=to] {
                    let Some(r) = slot.line.checked_sub(origin) else {
                        continue;
                    };
                    let Ok(r) = usize::try_from(r) else { continue };
                    if r < emu.rows() {
                        cells.push((r, slot.col));
                        if slot.wide {
                            cells.push((r, slot.col + 1));
                        }
                    }
                }
                let mut uri: String = text[..len].iter().collect();
                if local {
                    uri.insert_str(0, "http://");
                }
                return Some(LinkHit {
                    uri,
                    explicit: false,
                    cells,
                });
            }
        }
        i = j.max(i + 1);
    }
    None
}

/// O tamanho de `http://` ou `https://` no começo de `chars`, sem diferenciar maiúsculas; 0 se
/// não começa assim.
fn scheme_len(chars: &[(char, usize)]) -> usize {
    for scheme in ["https://", "http://"] {
        let n = scheme.len();
        if chars.len() >= n
            && chars[..n]
                .iter()
                .zip(scheme.chars())
                .all(|(&(c, _), s)| c.to_ascii_lowercase() == s)
        {
            return n;
        }
    }
    0
}

/// O tamanho do endereço local com porta no começo de `chars` (`localhost:3000`,
/// `127.x.y.z:3000`, `0.0.0.0:3000`, `[::1]:3000`; a porta vai de 1 a 65535 e não pode vir
/// colada numa letra ou número); 0 se não começa assim.
fn local_len(chars: &[(char, usize)]) -> usize {
    let text: String = chars.iter().take(64).map(|&(c, _)| c).collect();
    let lower = text.to_ascii_lowercase();
    let host = if lower.starts_with("localhost") {
        "localhost".len()
    } else if lower.starts_with("[::1]") {
        "[::1]".len()
    } else if lower.starts_with("0.0.0.0") {
        "0.0.0.0".len()
    } else if let Some(rest) = lower.strip_prefix("127.") {
        let mut length = "127.".len();
        let mut parts = rest.splitn(4, |c: char| !c.is_ascii_digit() && c != '.');
        let Some(addr) = parts.next() else { return 0 };
        let octets: Vec<&str> = addr.split('.').collect();
        if octets.len() < 3
            || octets[..3]
                .iter()
                .any(|o| o.is_empty() || o.len() > 3 || o.parse::<u8>().is_err())
        {
            return 0;
        }
        length += octets[..3].iter().map(|o| o.len()).sum::<usize>() + 2;
        length
    } else {
        return 0;
    };
    let bytes = lower.as_bytes();
    if bytes.get(host) != Some(&b':') {
        return 0;
    }
    let digits = bytes[host + 1..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    let end = host + 1 + digits;
    let valid_port = (1..=5).contains(&digits)
        && lower[host + 1..end]
            .parse::<u32>()
            .is_ok_and(|p| (1..=65535).contains(&p));
    let glued = lower[end..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_');
    if valid_port && !glued { end } else { 0 }
}

/// Se `c` termina a URL: espaço, aspas, `<`, `>`, crase ou caractere de controle.
fn ends_url(c: char) -> bool {
    c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '<' | '>' | '`')
}

/// O tamanho da URL sem a pontuação do fim da frase e sem o `)`, `]` ou `}` que não tem par
/// dentro dela: `(https://a.b/c)` sai sem o `)`, `https://pt.wikipedia.org/wiki/A_(b)` fica.
fn trimmed_len(chars: &[char]) -> usize {
    let mut len = chars.len();
    while len > 0 {
        let last = chars[len - 1];
        let opener = match last {
            ')' => '(',
            ']' => '[',
            '}' => '{',
            '.' | ',' | ';' | ':' | '!' | '?' => {
                len -= 1;
                continue;
            }
            _ => break,
        };
        let count = |target: char| chars[..len].iter().filter(|&&c| c == target).count();
        if count(last) > count(opener) {
            len -= 1;
        } else {
            break;
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(cols: usize, rows: usize, text: &str) -> Emulator {
        let mut t = Emulator::new(cols, rows, 100);
        t.feed(text.as_bytes());
        t
    }

    fn uri(t: &Emulator, row: usize, col: usize) -> Option<String> {
        find_link(t, row, col, true).map(|h| h.uri)
    }

    #[test]
    fn acha_a_url_no_meio_do_texto() {
        let t = term(40, 2, "veja https://exemplo.com/a?b=1 agora");
        assert_eq!(uri(&t, 0, 5).as_deref(), Some("https://exemplo.com/a?b=1"));
        assert_eq!(uri(&t, 0, 29).as_deref(), Some("https://exemplo.com/a?b=1"));
        assert_eq!(uri(&t, 0, 4), None, "o espaço antes não é do link");
        assert_eq!(uri(&t, 0, 30), None, "o espaço depois não é do link");
        assert_eq!(uri(&t, 0, 0), None);
    }

    #[test]
    fn sem_detectar_so_o_explicito_vale() {
        let t = term(40, 2, "https://exemplo.com");
        assert_eq!(find_link(&t, 0, 3, false), None);
        assert!(find_link(&t, 0, 3, true).is_some());
    }

    #[test]
    fn o_esquema_nao_diferencia_maiusculas_e_http_vale() {
        let t = term(40, 2, "HTTP://A.B/C e Https://d.e");
        assert_eq!(uri(&t, 0, 2).as_deref(), Some("HTTP://A.B/C"));
        assert_eq!(uri(&t, 0, 16).as_deref(), Some("Https://d.e"));
    }

    #[test]
    fn so_o_esquema_ou_colado_numa_palavra_nao_e_link() {
        let t = term(40, 2, "https:// x e xhttp://a.b");
        assert_eq!(uri(&t, 0, 2), None);
        assert_eq!(uri(&t, 0, 16), None);
    }

    #[test]
    fn o_endereco_local_sem_esquema_vira_http() {
        let t = term(60, 2, "abra localhost:3000/app agora");
        assert_eq!(uri(&t, 0, 5).as_deref(), Some("http://localhost:3000/app"));
        assert_eq!(uri(&t, 0, 22).as_deref(), Some("http://localhost:3000/app"));
        assert_eq!(uri(&t, 0, 4), None);
        assert_eq!(uri(&t, 0, 23), None);
        let hit = find_link(&t, 0, 5, true).unwrap();
        assert!(!hit.explicit);
        assert_eq!(find_link(&t, 0, 5, false), None);
        let t = term(60, 2, "127.0.0.1:8080.");
        assert_eq!(uri(&t, 0, 0).as_deref(), Some("http://127.0.0.1:8080"));
        let t = term(60, 2, "[::1]:5173 e 0.0.0.0:8000 e (127.1.2.3:9)");
        assert_eq!(uri(&t, 0, 2).as_deref(), Some("http://[::1]:5173"));
        assert_eq!(uri(&t, 0, 16).as_deref(), Some("http://0.0.0.0:8000"));
        assert_eq!(uri(&t, 0, 34).as_deref(), Some("http://127.1.2.3:9"));
        let t = term(60, 2, "\"LOCALHOST:80\"");
        assert_eq!(uri(&t, 0, 3).as_deref(), Some("http://LOCALHOST:80"));
    }

    #[test]
    fn o_endereco_local_mal_formado_nao_e_link() {
        for texto in [
            "user@localhost:22",
            "xlocalhost:3",
            "localhost:0",
            "localhost:70000",
            "localhost:123456",
            "localhost:3000abc",
            "127.0.0.1 sem porta",
            "127.0.0.300:80",
            "127.0.0:80",
            "localhost: 80",
        ] {
            let t = term(40, 2, texto);
            assert_eq!(uri(&t, 0, 8), None, "{texto}");
            assert_eq!(uri(&t, 0, 0), None, "{texto}");
        }
    }

    #[test]
    fn pontuacao_do_fim_sai() {
        for (texto, url) in [
            ("ver https://a.b/c.", "https://a.b/c"),
            ("ver https://a.b/c, ok", "https://a.b/c"),
            ("ver https://a.b/c?!", "https://a.b/c"),
            ("(https://a.b/c)", "https://a.b/c"),
            ("[https://a.b/c]", "https://a.b/c"),
            ("{https://a.b/c}", "https://a.b/c"),
            ("(veja https://a.b/c).", "https://a.b/c"),
            (
                "https://pt.wikipedia.org/wiki/A_(b)",
                "https://pt.wikipedia.org/wiki/A_(b)",
            ),
            (
                "https://pt.wikipedia.org/wiki/A_(b).",
                "https://pt.wikipedia.org/wiki/A_(b)",
            ),
            (
                "(https://pt.wikipedia.org/wiki/A_(b))",
                "https://pt.wikipedia.org/wiki/A_(b)",
            ),
            ("https://a.b/c;", "https://a.b/c"),
            ("https://a.b/c:", "https://a.b/c"),
        ] {
            let t = term(60, 2, texto);
            let col = texto.find("http").unwrap() + 8;
            assert_eq!(uri(&t, 0, col).as_deref(), Some(url), "{texto}");
        }
    }

    #[test]
    fn aspas_e_sinais_terminam_a_url() {
        for (texto, url) in [
            ("\"https://a.b/c\"", "https://a.b/c"),
            ("'https://a.b/c'", "https://a.b/c"),
            ("<https://a.b/c>", "https://a.b/c"),
            ("`https://a.b/c`", "https://a.b/c"),
            ("href=\"https://a.b/c\">", "https://a.b/c"),
        ] {
            let t = term(60, 2, texto);
            let col = texto.find("http").unwrap() + 8;
            assert_eq!(uri(&t, 0, col).as_deref(), Some(url), "{texto}");
        }
    }

    #[test]
    fn duas_urls_na_mesma_linha_sao_dois_links() {
        let t = term(60, 2, "https://a.b e https://c.d");
        assert_eq!(uri(&t, 0, 3).as_deref(), Some("https://a.b"));
        assert_eq!(uri(&t, 0, 20).as_deref(), Some("https://c.d"));
    }

    #[test]
    fn a_url_quebrada_pela_largura_e_uma_so() {
        // 10 colunas: a URL de 24 caracteres ocupa três linhas.
        let t = term(10, 4, "https://exemplo.com/abc");
        let hit = find_link(&t, 1, 4, true).expect("link");
        assert_eq!(hit.uri, "https://exemplo.com/abc");
        assert!(!hit.explicit);
        assert_eq!(hit.cells.len(), 23);
        assert!(hit.cells.contains(&(0, 0)));
        assert!(hit.cells.contains(&(2, 2)));
        assert_eq!(uri(&t, 2, 0).as_deref(), Some("https://exemplo.com/abc"));
    }

    #[test]
    fn linha_nova_do_programa_nao_junta() {
        let t = term(40, 3, "https://a.b/c\r\nd.e/f");
        assert_eq!(uri(&t, 0, 3).as_deref(), Some("https://a.b/c"));
        assert_eq!(uri(&t, 1, 0), None);
    }

    #[test]
    fn caracteres_largos_contam_como_uma_letra_e_duas_colunas() {
        let t = term(40, 2, "https://a.b/日本 fim");
        let hit = find_link(&t, 0, 12, true).expect("link");
        assert_eq!(hit.uri, "https://a.b/日本");
        // Coluna do segundo caractere largo (o espaço fica depois das 4 colunas dos dois).
        assert!(find_link(&t, 0, 15, true).is_some());
        assert!(hit.cells.contains(&(0, 12)) && hit.cells.contains(&(0, 13)));
        assert_eq!(find_link(&t, 0, 17, true), None);
    }

    #[test]
    fn acentos_combinados_entram_no_texto() {
        let t = term(40, 2, "https://a.b/e\u{301}x fim");
        assert_eq!(uri(&t, 0, 3).as_deref(), Some("https://a.b/e\u{301}x"));
    }

    #[test]
    fn o_link_explicito_junta_as_celulas_de_todas_as_linhas() {
        let t = term(
            10,
            3,
            "\x1b]8;id=k;https://x.org\x07ab\x1b]8;;\x07 \x1b]8;id=k;https://x.org\x07cd\x1b]8;;\x07\r\n\x1b]8;id=k;https://x.org\x07e\x1b]8;;\x07",
        );
        let hit = find_link(&t, 0, 0, false).expect("link");
        assert!(hit.explicit);
        assert_eq!(hit.uri, "https://x.org");
        assert_eq!(hit.cells, [(0, 0), (0, 1), (0, 3), (0, 4), (1, 0)]);
        // O texto do link não precisa parecer uma URL.
        assert_eq!(find_link(&t, 0, 2, true), None);
    }

    #[test]
    fn o_explicito_ganha_do_detectado() {
        let t = term(
            40,
            2,
            "\x1b]8;;file:///tmp/a\x07https://outro.com\x1b]8;;\x07",
        );
        let hit = find_link(&t, 0, 3, true).expect("link");
        assert_eq!(hit.uri, "file:///tmp/a");
        assert!(hit.explicit);
    }

    #[test]
    fn fora_da_tela_nao_ha_link() {
        let t = term(10, 2, "https://a.b");
        assert_eq!(find_link(&t, 0, 50, true), None);
    }

    #[test]
    fn a_url_no_historico_aparece_com_a_vista_rolada() {
        let mut t = Emulator::new(20, 2, 50);
        t.feed(b"https://a.b/c\r\nx\r\ny\r\nz");
        assert!(t.history_len() >= 1);
        t.set_display_offset(t.history_len());
        assert_eq!(uri(&t, 0, 2).as_deref(), Some("https://a.b/c"));
    }
}
