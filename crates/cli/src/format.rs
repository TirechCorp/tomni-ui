//! Texto para gente (e agentes) a partir das respostas do app.

use serde_json::Value;

/// Árvore de widgets indentada, um widget por linha:
///
/// ```text
/// form "Login" (0, 0, 390×844)
///   column (0, 0, 390×844)
///     edit #email placeholder="E-mail" text="" (24, 24, 342×44) rest
///     button #entrar "Entrar" accent (24, 136, 342×44) disabled
///     checkbox #termos "Li e aceito os termos" unchecked (24, 192, 342×44) rest
///     info_bar #aviso "Erro" error message="Tente de novo" (24, 248, 342×48) hidden
///     scroll_viewer #lista (24, 312, 342×500) scroll=(0, 120) content=342×2400 rest
///       button #item-1 "Primeiro" standard (24, 192, 342×44) rest scrolled-out
///     combo_box #uf placeholder="UF" text="SP" (24, 820, 342×44) rest
///       combo_box_dropdown #uf-lista (0, 0, 0×0) closed
///     slider #volume "Volume" value=40 range=0..=100 (24, 876, 342×56) rest
///     progress_bar #envio indeterminate (24, 944, 342×3)
///     tab_view #notas density=compact closable text="1" (24, 955, 342×200)
///       tab #notas-1 "Compras" selected (32, 955, 100×24) rest
///     card #tema "Tema" background=CardBackgroundFillColorDefault radius=8 (24, 960, 342×78) rest
///       text body text="Claro, escuro ou o do sistema." trimmed (41, 1001, 308×20)
///     button #config "Configurações" standard icon=Settings (24, 1046, 44×44) rest
///     icon icon=Home decorative (24, 1098, 20×20)
///     image #foto uniform_to_fill alt="Praia ao entardecer" (24, 1126, 320×180)
///     image #logo uniform error="não deu para ler logo.png: ..." (24, 1314, 0×0)
///     terminal #console "vim" campbell text="$ ls" grid=80×24 cursor=(4, 0) history=120 (24, 1322, 342×400) focused
///     terminal_panes #paineis layout=row[0,1] active=1 broadcast=none panes=[web-01*|web-02*] (24, 1730, 342×400) rest
///     data_grid #arquivos rows=200 sort="Nome crescente" selected=[a.txt,b.txt] (24, 1730, 342×400) rest
///     webview #ajuda "Sobre" url="fluent://app/sobre.html" back (24, 1730, 342×400) ready
///     split #divisao row value=240 (24, 2138, 342×300)
///     menu_bar_item #barra-arquivo "Arquivo" keys=Alt+A (28, 2446, 69×32) rest closed
///     menu_item #novo "Novo" keys=Ctrl+N (0, 0, 0×0) rest
///     text body text="app.log" context-menu (24, 2490, 60×20)
/// ```
///
/// `density=compact` (ou `standard`) aparece onde a densidade muda em relação ao pai; os
/// descendentes herdam. `closable` é um `TabView` com `.on_close` (as abas têm o X), e
/// `reorderable`, com `.on_reorder` (as abas mudam de ordem arrastando). `keys` são os atalhos
/// (`.accelerator`, o `.shortcut` do item de menu, o Alt e a letra da `MenuBar`), e
/// `context-menu` marca um widget com `.context_flyout`. No `split`, `row` ou `column` e o
/// tamanho do primeiro painel (`value`). `hidden` é um widget oculto (`.visible(false)`); `scrolled-out`, um rolado para fora da área
/// visível do `ScrollViewer` em volta; `trimmed`, um texto cortado com reticências
/// (`max_lines`). No `Terminal`, `grid` é o tamanho em colunas × linhas, `cursor` é
/// (coluna, linha), `history` são as linhas do histórico e `scrollback`, quantas delas a vista
/// subiu (só quando subiu). No `WebView`, `url` é a página, `back` e `forward` aparecem quando
/// dá para voltar e avançar no histórico, e o estado é `loading`, `ready` ou `failed` (com o
/// `error`). Um popup fechado sai como `closed`, sem os filhos; aberto,
/// como `open`, com eles.
pub fn tree(node: &Value) -> String {
    let mut out = String::new();
    write_node(node, 0, &mut out);
    out
}

fn write_node(node: &Value, depth: usize, out: &mut String) {
    let mut line = format!("{}{}", "  ".repeat(depth), str_of(&node["kind"]));
    let mut push = |part: String| {
        line.push(' ');
        line.push_str(&part);
    };
    if let Some(id) = node["id"].as_str() {
        push(format!("#{id}"));
    }
    if let Some(density) = node["density"].as_str() {
        push(format!("density={density}"));
    }
    for key in ["title", "label"] {
        if let Some(text) = node[key].as_str() {
            push(format!("{text:?}"));
        }
    }
    for key in ["variant", "severity"] {
        if let Some(value) = node[key].as_str() {
            push(value.to_owned());
        }
    }
    if let Some(icon) = node["icon"].as_str() {
        push(format!("icon={icon}"));
    }
    if node["decorative"] == true {
        push("decorative".into());
    }
    if let Some(alt) = node["alt"].as_str() {
        push(format!("alt={alt:?}"));
    }
    if let Some(error) = node["error"].as_str() {
        push(format!("error={error:?}"));
    }
    if let Some([w, h]) = node["natural_size"].as_array().map(Vec::as_slice) {
        push(format!("natural_size={}x{}", w, h));
    }
    if node["selected"] == true {
        push("selected".into());
    }
    if node["changed"] == true {
        push("changed".into());
    }
    if node["closable"] == true {
        push("closable".into());
    }
    if node["reorderable"] == true {
        push("reorderable".into());
    }
    match node["checked"].as_bool() {
        Some(true) => push("checked".into()),
        Some(false) => push("unchecked".into()),
        None => {}
    }
    if node["password"] == true {
        push("password".into());
    }
    if let Some(placeholder) = node["placeholder"].as_str() {
        push(format!("placeholder={placeholder:?}"));
    }
    for key in ["text", "message"] {
        if let Some(text) = node[key].as_str() {
            push(format!("{key}={text:?}"));
        }
    }
    if node["trimmed"] == true {
        push("trimmed".into());
    }
    if let Some(background) = node["background"].as_str() {
        push(format!("background={background}"));
    }
    if let Some(radius) = node["corner_radius"].as_f64() {
        push(format!("radius={}", number(radius)));
    }
    if let Some(value) = node["value"].as_f64() {
        push(format!("value={value}"));
    } else if matches!(
        node["kind"].as_str(),
        Some("progress_bar" | "progress_ring")
    ) {
        push("indeterminate".into());
    }
    if let Some(range) = node["range"].as_array() {
        let n = |i: usize| range[i].as_f64().unwrap_or_default();
        push(format!("range={}..={}", n(0), n(1)));
    }
    if let (Some(cols), Some(rows)) = (node["cols"].as_u64(), node["rows"].as_u64()) {
        push(format!("grid={cols}×{rows}"));
    }
    if let Some(cursor) = node["cursor"].as_array() {
        let n = |i: usize| cursor[i].as_u64().unwrap_or_default();
        push(format!("cursor=({}, {})", n(0), n(1)));
    }
    if let Some(history) = node["history"].as_array() {
        let n = |i: usize| history[i].as_u64().unwrap_or_default();
        push(format!("history={}", n(0)));
        if n(1) > 0 {
            push(format!("scrollback={}", n(1)));
        }
    }
    // O `DataGrid`: `rows=200 sort="Nome crescente" selected=[a,b]`.
    if node["kind"] == "data_grid" {
        if let Some(total) = node["items_total"].as_u64() {
            push(format!("rows={total}"));
        }
        if let Some(sort) = node["grid_sort"].as_str() {
            push(format!("sort={sort:?}"));
        }
        if let Some(filter) = node["filter"].as_str() {
            push(format!("filter={filter:?}"));
        }
        let selected: Vec<&str> = node["grid_selected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if !selected.is_empty() {
            push(format!("selected=[{}]", selected.join(",")));
        }
    }
    if let Some(layout) = node["layout"].as_str() {
        push(format!("layout={layout}"));
        if let Some(active) = node["active_pane"].as_u64() {
            push(format!("active={active}"));
        }
        if let Some(zoomed) = node["zoomed"].as_u64() {
            push(format!("zoomed={zoomed}"));
        }
        if let Some(broadcast) = node["broadcast"].as_str() {
            push(format!("broadcast={broadcast}"));
        }
        if node["identifying"].as_bool() == Some(true) {
            push("identifying".to_owned());
        }
        if let Some(highlighted) = node["highlighted"].as_u64() {
            push(format!("highlighted={highlighted}"));
        }
        // Cada painel com as chaves das abas, a escolhida com `*`: `panes=[a*,b|c*]`.
        if let Some(panes) = node["panes"].as_array() {
            let panes: Vec<String> = panes
                .iter()
                .map(|pane| {
                    let selected = pane["selected"].as_str();
                    let keys: Vec<String> = pane["tabs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|tab| tab[0].as_str())
                        .map(|key| match selected == Some(key) {
                            true => format!("{key}*"),
                            false => key.to_owned(),
                        })
                        .collect();
                    keys.join(",")
                })
                .collect();
            push(format!("panes=[{}]", panes.join("|")));
        }
    }
    if let Some(url) = node["url"].as_str() {
        push(format!("url={url:?}"));
    }
    if node["can_go_back"] == true {
        push("back".into());
    }
    if node["can_go_forward"] == true {
        push("forward".into());
    }
    if let Some(keys) = node["accelerators"].as_array() {
        let keys: Vec<&str> = keys.iter().filter_map(Value::as_str).collect();
        push(format!("keys={}", keys.join(",")));
    }
    if node["context_flyout"] == true {
        push("context-menu".into());
    }
    let r = |i: usize| node["rect"][i].as_f64().unwrap_or_default();
    push(format!(
        "({}, {}, {}×{})",
        number(r(0)),
        number(r(1)),
        number(r(2)),
        number(r(3))
    ));
    if let (Some(offset), Some(content)) = (node["offset"].as_array(), node["content"].as_array()) {
        let n = |v: &Value| number(v.as_f64().unwrap_or_default());
        push(format!("scroll=({}, {})", n(&offset[0]), n(&offset[1])));
        push(format!("content={}×{}", n(&content[0]), n(&content[1])));
    }
    if let Some(state) = node["state"].as_str() {
        push(state.to_owned());
    } else if node["enabled"] == false {
        push("disabled".into());
    }
    if node["focused"] == true && node["state"] != "focused" {
        push("focused".into());
    }
    let closed = node["open"] == false;
    if closed {
        push("closed".into());
    } else if node["open"] == true {
        push("open".into());
    } else if node["visible"] == false {
        push("hidden".into());
    } else if node["in_view"] == false {
        push("scrolled-out".into());
    }
    out.push_str(&line);
    out.push('\n');
    if closed {
        return;
    }
    for child in node["children"].as_array().into_iter().flatten() {
        write_node(child, depth + 1, out);
    }
}

/// Um problema do lint: `erro[touch-target] entrar: ...`.
pub fn issue(issue: &Value) -> String {
    let severity = if issue["severity"] == "warning" {
        "aviso"
    } else {
        "erro"
    };
    format!(
        "{severity}[{}] {}: {}",
        str_of(&issue["rule"]),
        str_of(&issue["widget"]),
        str_of(&issue["message"])
    )
}

/// Se o problema do lint é um erro (não um aviso).
pub fn is_error(issue: &Value) -> bool {
    issue["severity"] == "error"
}

/// `light`/`dark` em português.
pub fn theme(name: &str) -> &str {
    match name {
        "light" => "claro",
        "dark" => "escuro",
        other => other,
    }
}

fn str_of(value: &Value) -> &str {
    value.as_str().unwrap_or("?")
}

/// Sem `.0` nos inteiros.
fn number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn arvore_em_texto() {
        let node = json!({
            "kind": "form", "title": "Login", "rect": [0.0, 0.0, 390.0, 844.0], "enabled": true,
            "children": [{
                "kind": "button", "id": "entrar", "label": "Entrar", "variant": "accent",
                "rect": [24.0, 136.5, 342.0, 44.0], "state": "disabled", "enabled": false
            }, {
                "kind": "checkbox", "id": "termos", "label": "Termos", "checked": false,
                "rect": [24.0, 190.0, 342.0, 44.0], "state": "rest", "enabled": true
            }, {
                "kind": "info_bar", "title": "Erro", "message": "Tente de novo",
                "severity": "error", "rect": [0.0, 0.0, 0.0, 0.0], "enabled": true,
                "visible": false
            }, {
                "kind": "scroll_viewer", "id": "lista", "rect": [0.0, 300.0, 390.0, 500.0],
                "offset": [0.0, 120.0], "content": [390.0, 2400.0], "state": "rest",
                "enabled": true,
                "children": [{
                    "kind": "button", "id": "um", "label": "Um", "variant": "standard",
                    "rect": [0.0, 180.0, 390.0, 44.0], "state": "rest", "enabled": true,
                    "in_view": false
                }]
            }]
        });
        assert_eq!(
            tree(&node),
            "form \"Login\" (0, 0, 390×844)\n  \
             button #entrar \"Entrar\" accent (24, 136.5, 342×44) disabled\n  \
             checkbox #termos \"Termos\" unchecked (24, 190, 342×44) rest\n  \
             info_bar \"Erro\" error message=\"Tente de novo\" (0, 0, 0×0) hidden\n  \
             scroll_viewer #lista (0, 300, 390×500) scroll=(0, 120) content=390×2400 rest\n    \
             button #um \"Um\" standard (0, 180, 390×44) rest scrolled-out\n"
        );
    }

    #[test]
    fn abas_que_fecham() {
        let node = json!({
            "kind": "tab_view", "id": "notas", "text": "1", "closable": true,
            "tabs": [["1", "Compras"]], "rect": [0.0, 0.0, 390.0, 300.0], "enabled": true,
            "children": [{
                "kind": "tab", "id": "notas-1", "label": "Compras", "selected": true,
                "rect": [8.0, 0.0, 100.0, 24.0], "state": "rest", "enabled": true
            }]
        });
        assert_eq!(
            tree(&node),
            "tab_view #notas closable text=\"1\" (0, 0, 390×300)\n  \
             tab #notas-1 \"Compras\" selected (8, 0, 100×24) rest\n"
        );
    }

    #[test]
    fn popups_em_texto() {
        let item = |id: &str, label: &str, selected: bool| {
            json!({
                "kind": "combo_box_item", "id": id, "label": label, "selected": selected,
                "rect": [0.0, 0.0, 100.0, 48.0], "state": "rest", "enabled": true
            })
        };
        let node = json!({
            "kind": "column", "rect": [0.0, 0.0, 390.0, 844.0], "enabled": true,
            "children": [{
                "kind": "combo_box", "id": "uf", "text": "SP", "placeholder": "UF",
                "rect": [24.0, 24.0, 120.0, 44.0], "state": "rest", "enabled": true,
                "children": [{
                    "kind": "combo_box_dropdown", "id": "uf-lista", "open": true,
                    "rect": [24.0, 72.0, 120.0, 98.0], "enabled": true,
                    "children": [item("uf-rj", "RJ", false), item("uf-sp", "SP", true)]
                }]
            }, {
                "kind": "content_dialog", "id": "excluir", "title": "Excluir?", "open": false,
                "rect": [0.0, 0.0, 0.0, 0.0], "enabled": true, "visible": false,
                "children": [{ "kind": "scroll_viewer", "rect": [0.0, 0.0, 0.0, 0.0] }]
            }]
        });
        assert_eq!(
            tree(&node),
            "column (0, 0, 390×844)\n  \
             combo_box #uf placeholder=\"UF\" text=\"SP\" (24, 24, 120×44) rest\n    \
             combo_box_dropdown #uf-lista (24, 72, 120×98) open\n      \
             combo_box_item #uf-rj \"RJ\" (0, 0, 100×48) rest\n      \
             combo_box_item #uf-sp \"SP\" selected (0, 0, 100×48) rest\n  \
             content_dialog #excluir \"Excluir?\" (0, 0, 0×0) closed\n"
        );
    }

    #[test]
    fn valores_em_texto() {
        let node = json!({
            "kind": "column", "rect": [0.0, 0.0, 390.0, 844.0], "enabled": true,
            "children": [{
                "kind": "slider", "id": "volume", "label": "Volume", "value": 40.0,
                "range": [0.0, 100.0], "rect": [24.0, 24.0, 342.0, 56.0], "state": "rest",
                "enabled": true
            }, {
                "kind": "progress_bar", "id": "envio", "value": 0.25,
                "rect": [24.0, 92.0, 342.0, 3.0], "enabled": true
            }, {
                "kind": "progress_ring", "id": "anel", "rect": [24.0, 107.0, 32.0, 32.0],
                "enabled": true
            }]
        });
        assert_eq!(
            tree(&node),
            "column (0, 0, 390×844)\n  \
             slider #volume \"Volume\" value=40 range=0..=100 (24, 24, 342×56) rest\n  \
             progress_bar #envio value=0.25 (24, 92, 342×3)\n  \
             progress_ring #anel indeterminate (24, 107, 32×32)\n"
        );
    }

    #[test]
    fn problema_em_texto() {
        let i = json!({"rule": "missing-id", "severity": "warning", "widget": "button", "message": "x"});
        assert_eq!(issue(&i), "aviso[missing-id] button: x");
        assert!(!is_error(&i));
    }

    #[test]
    fn card_e_texto_cortado_em_texto() {
        let node = json!({
            "kind": "card", "id": "tema", "label": "Tema",
            "background": "CardBackgroundFillColorDefault", "corner_radius": 8.0,
            "rect": [24.0, 68.0, 342.0, 78.0], "state": "rest", "enabled": true,
            "children": [{
                "kind": "text", "variant": "body", "text": "Claro, escuro ou o do sistema.",
                "trimmed": true, "rect": [41.0, 109.0, 308.0, 20.0], "enabled": true
            }]
        });
        assert_eq!(
            tree(&node),
            "card #tema \"Tema\" background=CardBackgroundFillColorDefault radius=8 \
             (24, 68, 342×78) rest\n  \
             text body text=\"Claro, escuro ou o do sistema.\" trimmed (41, 109, 308×20)\n"
        );
    }

    #[test]
    fn terminal_em_texto() {
        let node = json!({
            "kind": "terminal", "id": "console", "title": "vim", "variant": "campbell",
            "text": "$ ls", "cols": 80, "rows": 24, "cursor": [4, 0], "history": [120, 3],
            "rect": [24.0, 24.0, 342.0, 400.0], "state": "focused", "enabled": true,
            "focused": true
        });
        assert_eq!(
            tree(&node),
            "terminal #console \"vim\" campbell text=\"$ ls\" grid=80×24 cursor=(4, 0) \
             history=120 scrollback=3 (24, 24, 342×400) focused\n"
        );
    }

    #[test]
    fn terminal_panes_em_texto() {
        let node = json!({
            "kind": "terminal_panes", "id": "paineis", "layout": "row[0,1]",
            "active_pane": 1, "zoomed": 0, "broadcast": "group:web-01,web-03",
            "identifying": true, "highlighted": 1,
            "panes": [
                {"tabs": [["web-01", "web-01"]], "selected": "web-01"},
                {"tabs": [["web-02", "web-02"], ["web-03", "web-03"]], "selected": "web-03"}
            ],
            "rect": [16.0, 100.0, 358.0, 500.0], "state": "rest", "enabled": true
        });
        assert_eq!(
            tree(&node),
            "terminal_panes #paineis layout=row[0,1] active=1 zoomed=0 \
             broadcast=group:web-01,web-03 identifying highlighted=1 \
             panes=[web-01*|web-02,web-03*] \
             (16, 100, 358×500) rest\n"
        );
    }

    #[test]
    fn data_grid_em_texto() {
        let node = json!({
            "kind": "data_grid", "id": "arquivos", "items_total": 200,
            "grid_sort": "Nome crescente", "grid_selected": ["a.txt", "b.txt"],
            "rect": [0.0, 40.0, 600.0, 300.0], "state": "rest", "enabled": true
        });
        assert_eq!(
            tree(&node),
            "data_grid #arquivos rows=200 sort=\"Nome crescente\" selected=[a.txt,b.txt] \
             (0, 40, 600×300) rest\n"
        );
    }

    #[test]
    fn webview_em_texto() {
        let node = json!({
            "kind": "webview", "id": "ajuda", "title": "Sobre",
            "url": "fluent://app/sobre.html", "can_go_back": true, "can_go_forward": false,
            "rect": [24.0, 24.0, 342.0, 400.0], "state": "ready", "enabled": true
        });
        assert_eq!(
            tree(&node),
            "webview #ajuda \"Sobre\" url=\"fluent://app/sobre.html\" back \
             (24, 24, 342×400) ready\n"
        );
        let falhou = json!({
            "kind": "webview", "id": "ajuda", "url": "https://exemplo.com/",
            "error": "sem conexão", "can_go_back": false, "can_go_forward": true,
            "rect": [0.0, 0.0, 200.0, 150.0], "state": "failed", "enabled": true
        });
        assert_eq!(
            tree(&falhou),
            "webview #ajuda error=\"sem conexão\" url=\"https://exemplo.com/\" forward \
             (0, 0, 200×150) failed\n"
        );
    }

    #[test]
    fn divisor_atalhos_e_menu_de_contexto_em_texto() {
        let node = json!({
            "kind": "split", "id": "divisao", "variant": "row", "value": 240.0,
            "rect": [0.0, 0.0, 800.0, 400.0], "enabled": true,
            "children": [
                {
                    "kind": "menu_bar_item", "id": "barra-arquivo", "label": "Arquivo",
                    "accelerators": ["Alt+A"], "open": false, "state": "rest",
                    "rect": [4.0, 4.0, 69.0, 32.0], "enabled": true
                },
                {
                    "kind": "tab_view", "id": "logs", "text": "app", "reorderable": true,
                    "rect": [0.0, 40.0, 400.0, 300.0], "enabled": true
                },
                {
                    "kind": "text", "variant": "body", "text": "app.log", "context_flyout": true,
                    "rect": [8.0, 360.0, 60.0, 20.0], "enabled": true
                }
            ]
        });
        assert_eq!(
            tree(&node),
            "split #divisao row value=240 (0, 0, 800×400)\n  \
             menu_bar_item #barra-arquivo \"Arquivo\" keys=Alt+A (4, 4, 69×32) rest closed\n  \
             tab_view #logs reorderable text=\"app\" (0, 40, 400×300)\n  \
             text body text=\"app.log\" context-menu (8, 360, 60×20)\n"
        );
    }
}
