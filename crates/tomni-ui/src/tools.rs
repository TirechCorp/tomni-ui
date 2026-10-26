//! Modo de ferramentas: o app atende a CLI `tomni-ui` e o servidor MCP sem abrir janela.
//!
//! Com [`ENV`]` = 1`, [`App::run`](crate::App::run) lê pedidos em JSON, um por linha, da entrada
//! padrão, e responde na saída padrão com linhas que começam com [`PREFIX`] (o resto da saída
//! é do app e é ignorado). A primeira resposta, sem pedido, lista as telas. A tela é montada no
//! [`Ui`] de sempre e rasterizada no backend CPU determinístico, o mesmo dos snapshots.
//!
//! Pedidos (`cmd`): `open` (`view`, `width`, `height`, `theme`, todos opcionais), `resize`,
//! `theme`, `screenshot` (`path` do PNG), `tree`, `lint` e `act` (`actions`, com `click`,
//! `double_click`, `middle_click`, `hover`, `click_at`, `type`, `key`, `ime_preedit`, `ime_commit`,
//! `scroll`, `select`, `set_value` e `wait`). Respostas:
//! `{"ok": true, ...}` ou `{"ok": false, "error": "..."}`.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tomni_ui_core::kurbo::{Point, Vec2};
use tomni_ui_core::{Key, Modifiers, NodeId, PointerKind};
use tomni_ui_reactive::untrack;
use tomni_ui_render::CpuCanvas;
use tomni_ui_theme::{FluentDrawer, Theme};

use crate::{App, Ui};

/// Variável de ambiente que liga o modo de ferramentas.
pub const ENV: &str = "TOMNI_UI_TOOLS";

/// Começo de cada linha de resposta na saída padrão.
pub const PREFIX: &str = "@tomni-ui ";

/// Versão do protocolo, na primeira resposta.
pub const PROTOCOL: u32 = 19;

pub(crate) fn requested() -> bool {
    std::env::var_os(ENV).is_some_and(|value| value == "1")
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Open {
        view: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
        theme: Option<ThemeName>,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Theme {
        theme: ThemeName,
    },
    Screenshot {
        path: PathBuf,
    },
    Tree,
    Lint,
    Act {
        actions: Vec<Action>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ThemeName {
    Light,
    Dark,
}

impl From<ThemeName> for Theme {
    fn from(name: ThemeName) -> Self {
        match name {
            ThemeName::Light => Self::Light,
            ThemeName::Dark => Self::Dark,
        }
    }
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Light => "light",
        Theme::Dark => "dark",
    }
}

/// Interação, como o `TestApp` faria.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    /// Clique no centro do widget.
    Click { id: String },
    /// Duplo clique no centro do widget.
    DoubleClick { id: String },
    /// Clique do meio no centro do widget (fecha uma aba do `TabView`).
    MiddleClick { id: String },
    /// Mouse sobre o centro do widget.
    Hover { id: String },
    /// Clique num ponto da janela.
    ClickAt { x: f64, y: f64 },
    /// Arrasta com o mouse do centro do widget `id` até o centro do widget `to` ou até o
    /// ponto `x`, `y` da janela: o divisor do `Split`, o `Slider` e as abas de um `TabView`
    /// com `.on_reorder`.
    Drag {
        id: String,
        to: Option<String>,
        x: Option<f64>,
        y: Option<f64>,
    },
    /// Abre o menu de contexto do widget `id` (`.context_flyout`, ou o
    /// `.tab_context_flyout` na aba) com o botão direito no centro dele.
    Context { id: String },
    /// Clica no campo `id`, vai ao fim e digita; sem `id`, digita no widget com foco. Num
    /// `Terminal`, escreve na conexão: não vai ao fim, `\n` é o Enter e `\t` é o Tab.
    Type { id: Option<String>, text: String },
    /// Pressiona e solta uma tecla: `enter`, `tab`, `escape`, `backspace`, `delete`, `left`,
    /// `right`, `up`, `down`, `home`, `end`, `pageup`, `pagedown`, `space` ou uma letra
    /// (atalhos).
    Key {
        key: String,
        #[serde(default)]
        modifiers: Vec<String>,
    },
    /// Composição do IME em andamento.
    ImePreedit { text: String },
    /// O IME confirma o texto.
    ImeCommit { text: String },
    /// Rola o `ScrollViewer` `id` em `dx`, `dy` pixels, como a roda do mouse sobre ele
    /// (positivo mostra o que está mais abaixo ou à direita).
    Scroll {
        id: String,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
    /// Escolhe `option` no `ComboBox` `id` (abre a lista e clica no item), no `RadioButtons`
    /// `id` (clica na opção), no `TabView` `id` (clica na aba com esse título) ou no
    /// `NavigationView` `id` (abre o painel e o grupo, se preciso, e clica no item com esse
    /// rótulo).
    Select { id: String, option: String },
    /// Leva o `Slider` `id` a `value`: clica no trilho, no ponto do valor.
    SetValue { id: String, value: f64 },
    /// Espera `ms` milissegundos (a dica de um `hover` abre em 500).
    Wait { ms: u64 },
}

struct Session {
    view: String,
    theme: Theme,
    ui: Ui,
}

struct Server {
    app: App,
    session: Option<Session>,
}

type Reply = Result<Value, String>;

/// Atende os pedidos até a entrada padrão fechar.
pub(crate) fn serve(app: App) {
    let views: Vec<&str> = app.views.iter().map(|v| v.name.as_str()).collect();
    let hello = json!({
        "ok": true,
        "protocol": PROTOCOL,
        "views": views,
        "width": app.options.width,
        "height": app.options.height,
        "theme": app.theme.map(theme_name),
    });
    let mut server = Server { app, session: None };
    if send(&hello).is_err() {
        return;
    }
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = serde_json::from_str::<Request>(&line)
            .map_err(|e| format!("pedido inválido: {e}"))
            .and_then(|request| server.handle(request));
        let response = match reply {
            Ok(mut value) => {
                value["ok"] = Value::Bool(true);
                value
            }
            Err(error) => json!({ "ok": false, "error": error }),
        };
        if send(&response).is_err() {
            break;
        }
    }
}

fn send(value: &Value) -> io::Result<()> {
    let mut out = io::stdout().lock();
    writeln!(out, "{PREFIX}{value}")?;
    out.flush()
}

impl Server {
    fn handle(&mut self, request: Request) -> Reply {
        match request {
            Request::Open {
                view,
                width,
                height,
                theme,
            } => self.open(view, width, height, theme.map(Theme::from)),
            Request::Resize { width, height } => {
                check_size(width, height)?;
                let ui = &mut self.session()?.ui;
                ui.resize(width, height);
                ui.frame();
                Ok(json!({}))
            }
            Request::Theme { theme } => {
                let session = self.session()?;
                session.theme = theme.into();
                session.ui.set_drawer(FluentDrawer::new(session.theme));
                session.ui.frame();
                Ok(json!({}))
            }
            Request::Screenshot { path } => {
                let ui = &mut self.session()?.ui;
                pump(ui);
                screenshot(ui, path)
            }
            Request::Tree => {
                let session = self.session()?;
                pump(&mut session.ui);
                Ok(json!({
                    "view": session.view,
                    "size": session.ui.size(),
                    "theme": theme_name(session.theme),
                    "tree": session.ui.inspect(),
                }))
            }
            Request::Lint => {
                let ui = &mut self.session()?.ui;
                pump(ui);
                Ok(json!({ "issues": ui.lint() }))
            }
            Request::Act { actions } => {
                let ui = &mut self.session()?.ui;
                pump(ui);
                let mut notes = Vec::new();
                for (i, action) in actions.into_iter().enumerate() {
                    act(ui, action, &mut notes).map_err(|e| format!("ação {}: {e}", i + 1))?;
                }
                Ok(json!({ "repaints": ui.repaints_since_last(), "notes": notes }))
            }
        }
    }

    /// Sessão aberta; sem nenhuma, abre a primeira tela com o tamanho e o tema do app.
    fn session(&mut self) -> Result<&mut Session, String> {
        if self.session.is_none() {
            self.open(None, None, None, None)?;
        }
        Ok(self.session.as_mut().expect("aberta acima"))
    }

    fn open(
        &mut self,
        view: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
        theme: Option<Theme>,
    ) -> Reply {
        let width = width.unwrap_or(self.app.options.width);
        let height = height.unwrap_or(self.app.options.height);
        check_size(width, height)?;
        let theme = theme.or(self.app.theme).unwrap_or_default();
        let named = match &view {
            None => &self.app.views[0],
            Some(name) => self
                .app
                .views
                .iter()
                .find(|v| &v.name == name)
                .ok_or_else(|| {
                    let names: Vec<&str> = self.app.views.iter().map(|v| v.name.as_str()).collect();
                    format!(
                        "nenhuma tela {name:?}; telas: {}. Registre outras com `App::view`",
                        names.join(", ")
                    )
                })?,
        };
        // O `Ui` anterior sai antes, para não haver dois escopos reativos vivos.
        self.session = None;
        let mut ui = (named.mount)(width, height, theme);
        pump(&mut ui);
        ui.repaints_since_last();
        let reply = json!({
            "view": named.name,
            "size": [width, height],
            "theme": theme_name(theme),
        });
        self.session = Some(Session {
            view: named.name.clone(),
            theme,
            ui,
        });
        Ok(reply)
    }
}

/// Atende o que chegou de fora desde o último pedido, como o loop da janela faria ao ser
/// acordado: a saída que a conexão de um `Terminal` já entregou.
fn pump(ui: &mut Ui) {
    ui.advance(Duration::ZERO);
    ui.frame();
}

fn check_size(width: u32, height: u32) -> Result<(), String> {
    let max = u32::from(u16::MAX);
    if width == 0 || height == 0 || width > max || height > max {
        return Err(format!(
            "tamanho {width}x{height} inválido: de 1 a {max} pixels em cada lado"
        ));
    }
    Ok(())
}

fn screenshot(ui: &Ui, path: PathBuf) -> Reply {
    let (width, height) = ui.size();
    let to_u16 = |v: u32| u16::try_from(v).expect("check_size limita a u16");
    let mut canvas = CpuCanvas::deterministic(to_u16(width), to_u16(height));
    ui.render(&mut canvas);
    let pixmap = canvas.render(ui.drawer().tokens().solid_background_base);
    let png = pixmap
        .into_png()
        .map_err(|e| format!("falha ao codificar PNG: {e}"))?;
    std::fs::write(&path, png).map_err(|e| format!("falha ao gravar {}: {e}", path.display()))?;
    Ok(json!({ "path": path, "size": [width, height] }))
}

/// Se `id` é de um `TerminalPanes`, o id do terminal do painel ativo, que é quem recebe o que
/// se digita. `None` para qualquer outro widget.
#[cfg(feature = "terminal")]
fn panes_target(ui: &Ui, id: &str) -> Result<Option<String>, String> {
    let n = node(ui, id)?;
    let tree = ui.tree();
    let widget = tree.widget(n);
    let Some(panes) = widget
        .as_any()
        .and_then(|any| any.downcast_ref::<tomni_ui_terminal::TerminalPanes>())
    else {
        return Ok(None);
    };
    panes
        .active_terminal()
        .map(Some)
        .ok_or_else(|| format!("o painel ativo de {id} não tem nenhuma sessão"))
}

#[cfg(not(feature = "terminal"))]
fn panes_target(_: &Ui, _: &str) -> Result<Option<String>, String> {
    Ok(None)
}

/// Widget com o id `id`, ou um erro com os ids que existem.
fn node(ui: &Ui, id: &str) -> Result<NodeId, String> {
    ui.find(id).ok_or_else(|| {
        let tree = ui.tree();
        let ids: Vec<String> = tree
            .ids()
            .filter_map(|n| tree.widget(n).common().id.clone())
            .collect();
        if ids.is_empty() {
            format!("nenhum widget com id {id:?}; a tela não tem ids (use `.id(\"...\")`)")
        } else {
            format!("nenhum widget com id {id:?}; ids: {}", ids.join(", "))
        }
    })
}

/// Centro da área clicável de `id`, ou um erro se ele está oculto. Antes, rola os
/// `ScrollViewer` em volta até ele aparecer, como uma pessoa faria.
fn target(ui: &mut Ui, id: NodeId, name: &str) -> Result<Point, String> {
    if !ui.shown(id) {
        return Err(format!(
            "{name} está oculto (`.visible(false)` nele ou num contêiner em volta) e não recebe o ponteiro"
        ));
    }
    ui.bring_into_view(id);
    ui.frame();
    Ok(ui.hit_rect(id).center())
}

/// Avisa quando um popup aberto está na frente de `id`: o clique vai para o popup (ou fecha
/// um que fecha com clique fora) e não chega a `id`.
fn note_covered(ui: &Ui, id: NodeId, name: &str, notes: &mut Vec<String>) {
    let tree = ui.tree();
    if tree.in_scope(id) {
        return;
    }
    let top = tree
        .open_layers()
        .into_iter()
        .rev()
        .find(|&layer| tree.layer(layer).is_some_and(|l| l.input));
    if let Some(top) = top {
        let popup = tree.name(top);
        let effect = if tree.layer(top).is_some_and(|l| l.light_dismiss) {
            "o clique só fechou o popup"
        } else {
            "o clique não chegou a ele"
        };
        notes.push(format!(
            "{name} está atrás do popup {popup}, aberto: {effect}; feche o popup antes (Esc ou um botão dele)"
        ));
    }
}

/// Avisa quando a ação vai num widget desabilitado, que a ignora.
fn note_disabled(ui: &Ui, id: NodeId, name: &str, notes: &mut Vec<String>) {
    let tree = ui.tree();
    if !untrack(|| tree.widget(id).enabled()) {
        notes.push(format!("{name} está desabilitado e ignorou a ação"));
    }
}

fn act(ui: &mut Ui, action: Action, notes: &mut Vec<String>) -> Result<(), String> {
    match action {
        Action::Click { id } => {
            let n = node(ui, &id)?;
            let point = target(ui, n, &id)?;
            note_disabled(ui, n, &id, notes);
            note_covered(ui, n, &id, notes);
            click_at(ui, point, 1);
        }
        Action::DoubleClick { id } => {
            let n = node(ui, &id)?;
            let center = target(ui, n, &id)?;
            note_disabled(ui, n, &id, notes);
            note_covered(ui, n, &id, notes);
            click_at(ui, center, 1);
            click_at(ui, center, 2);
        }
        Action::MiddleClick { id } => {
            let n = node(ui, &id)?;
            let point = target(ui, n, &id)?;
            note_disabled(ui, n, &id, notes);
            note_covered(ui, n, &id, notes);
            ui.pointer_move(point, PointerKind::Mouse);
            ui.frame();
            ui.middle_down(point);
            ui.frame();
            ui.middle_up(point);
            ui.frame();
        }
        Action::Hover { id } => {
            let n = node(ui, &id)?;
            let point = target(ui, n, &id)?;
            ui.pointer_move(point, PointerKind::Mouse);
            ui.frame();
        }
        Action::ClickAt { x, y } => click_at(ui, Point::new(x, y), 1),
        Action::Drag { id, to, x, y } => drag(ui, &id, to.as_deref(), x.zip(y), notes)?,
        Action::Context { id } => {
            let n = node(ui, &id)?;
            let tree = ui.tree();
            let mut owner = Some(n);
            while let Some(o) = owner
                && tree.context_flyout(o).is_none()
            {
                owner = tree.parent(o);
            }
            let Some(menu) = owner.and_then(|o| tree.context_flyout(o)) else {
                return Err(format!(
                    "{id} não tem menu de contexto (`.context_flyout`), nem um contêiner em volta"
                ));
            };
            let point = target(ui, n, &id)?;
            note_covered(ui, n, &id, notes);
            ui.pointer_move(point, PointerKind::Mouse);
            ui.frame();
            ui.secondary_down(point);
            ui.frame();
            ui.secondary_up(point);
            ui.frame();
            if ui.tree().shown(menu) {
                notes.push(format!("menu {} aberto", ui.tree().name(menu)));
            }
        }
        Action::Type { id, text } => {
            let terminal = if let Some(id) = id {
                // Num `TerminalPanes`, o texto vai ao terminal do painel ativo (e, pelo
                // roteador do widget, ao envio para vários, se estiver ligado).
                let id = panes_target(ui, &id)?.unwrap_or(id);
                let n = node(ui, &id)?;
                let terminal = ui.tree().widget(n).kind() == "terminal";
                if !terminal && ui.tree().widget(n).text_input().is_none() {
                    return Err(format!("{id} não é um campo de texto"));
                }
                let point = target(ui, n, &id)?;
                note_disabled(ui, n, &id, notes);
                note_covered(ui, n, &id, notes);
                click_at(ui, point, 1);
                // No terminal, o End iria ao programa.
                if !terminal {
                    key(ui, Key::End, Modifiers::NONE);
                }
                terminal
            } else if ui.text_input().is_none() {
                return Err("nenhum campo de texto com foco; passe `id`".into());
            } else {
                let tree = ui.tree();
                tree.focused()
                    .is_some_and(|n| tree.widget(n).kind() == "terminal")
            };
            let mut buf = [0; 4];
            for c in text.chars() {
                match c {
                    '\n' if terminal => key(ui, Key::Enter, Modifiers::NONE),
                    '\t' if terminal => key(ui, Key::Tab, Modifiers::NONE),
                    _ => {
                        ui.type_text(c.encode_utf8(&mut buf));
                        ui.frame();
                    }
                }
                if terminal {
                    // O eco que já chegou entra na resposta.
                    pump(ui);
                }
            }
        }
        Action::Key {
            key: name,
            modifiers,
        } => {
            let k = parse_key(&name)?;
            let mut mods = Modifiers::NONE;
            for m in &modifiers {
                mods = mods | parse_modifier(m)?;
            }
            if k == Key::Tab && mods.shift {
                ui.focus_prev();
                ui.frame();
            } else {
                key(ui, k, mods);
            }
        }
        Action::ImePreedit { text } => {
            ui.ime(tomni_ui_core::Ime::Preedit { text, cursor: None });
            ui.frame();
        }
        Action::ImeCommit { text } => {
            ui.ime(tomni_ui_core::Ime::Commit(text));
            ui.frame();
        }
        Action::Scroll { id, dx, dy } => {
            let n = node(ui, &id)?;
            let Some(scroller) = ui.tree().scroller(n) else {
                return Err(format!(
                    "{id} não é um `ScrollViewer`; para mostrar um widget rolado para fora, aja \
                     nele direto (click, type, hover rolam até ele)"
                ));
            };
            if !scroller.can_scroll() {
                notes.push(format!("{id} não rola: o conteúdo cabe na área visível"));
            }
            ui.scroll_by(n, Vec2::new(dx, dy));
            ui.frame();
            let offset = ui
                .tree()
                .scroller(n)
                .map(|s| s.offset())
                .unwrap_or_default();
            notes.push(format!("{id} rolado até ({}, {})", offset.x, offset.y));
        }
        Action::Select { id, option } => select(ui, &id, &option, notes)?,
        Action::SetValue { id, value } => {
            let n = node(ui, &id)?;
            let number_box = ui.tree().widget(n).kind() == "number_box";
            if !number_box && untrack(|| ui.tree().widget(n).range()).is_none() {
                return Err(format!("{id} não é um `Slider` nem um `NumberBox`"));
            }
            let point = target(ui, n, &id)?;
            note_disabled(ui, n, &id, notes);
            note_covered(ui, n, &id, notes);
            if number_box {
                // Como uma pessoa: seleciona tudo, digita com vírgula e confirma.
                click_at(ui, point, 1);
                key(ui, Key::Char('a'), Modifiers::COMMAND);
                ui.type_text(&value.to_string().replace('.', ","));
                ui.frame();
                key(ui, Key::Enter, Modifiers::NONE);
            } else if let Some(point) = ui.slider_point(n, value) {
                click_at(ui, point, 1);
            }
            let now = untrack(|| ui.tree().widget(n).text()).unwrap_or_default();
            notes.push(format!("{id} ficou em {now}"));
        }
        Action::Wait { ms } => {
            ui.advance(std::time::Duration::from_millis(ms));
            ui.frame();
        }
    }
    Ok(())
}

/// Escolhe `option` no grupo `id`: abre a lista do `ComboBox` e clica no item, clica na
/// opção do `RadioButtons` ou na aba do `TabView`, ou abre o que for preciso do
/// `NavigationView` e clica no item.
fn select(ui: &mut Ui, id: &str, option: &str, notes: &mut Vec<String>) -> Result<(), String> {
    let n = node(ui, id)?;
    let kind = ui.tree().widget(n).kind();
    if !matches!(
        kind,
        "combo_box" | "radio_buttons" | "tab_view" | "navigation_view"
    ) {
        return Err(format!(
            "{id} não é um `ComboBox`, um `RadioButtons`, um `TabView` nem um `NavigationView`"
        ));
    }
    if kind == "navigation_view" {
        // Abre o painel e os grupos em volta do item, um clique por vez.
        for _ in 0..5 {
            let (step, last) = ui.navigation_step(n, option).map_err(|labels| {
                format!(
                    "{id} não tem o item {option:?}; itens: {}",
                    labels.join(", ")
                )
            })?;
            let point = target(ui, step, option)?;
            note_covered(ui, step, option, notes);
            click_at(ui, point, 1);
            if last {
                return Ok(());
            }
        }
        return Err(format!("o item {option:?} de {id} não apareceu"));
    }
    let item = ui.part(n, option).map_err(|labels| {
        format!(
            "{id} não tem a opção {option:?}; opções: {}",
            labels.join(", ")
        )
    })?;
    if kind != "combo_box" {
        let point = target(ui, item, option)?;
        note_disabled(ui, item, option, notes);
        note_covered(ui, item, option, notes);
        click_at(ui, point, 1);
        return Ok(());
    }
    let point = target(ui, n, id)?;
    note_disabled(ui, n, id, notes);
    note_covered(ui, n, id, notes);
    if !untrack(|| ui.tree().widget(n).enabled()) {
        return Ok(());
    }
    click_at(ui, point, 1);
    let point = target(ui, item, option)?;
    click_at(ui, point, 1);
    Ok(())
}

/// Arrasta do centro de `id` até o centro de `to` ou até `point`.
fn drag(
    ui: &mut Ui,
    id: &str,
    to: Option<&str>,
    point: Option<(f64, f64)>,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let n = node(ui, id)?;
    let from = target(ui, n, id)?;
    note_disabled(ui, n, id, notes);
    note_covered(ui, n, id, notes);
    let end = match (to, point) {
        (Some(to), None) => {
            let t = node(ui, to)?;
            if !ui.shown(t) {
                return Err(format!("{to} está oculto e não recebe o item"));
            }
            ui.hit_rect(t).center()
        }
        (None, Some((x, y))) => Point::new(x, y),
        _ => return Err("passe `to` (um id) ou `x` e `y` (um ponto da janela)".into()),
    };
    ui.pointer_move(from, PointerKind::Mouse);
    ui.frame();
    ui.pointer_down(from, PointerKind::Mouse, 1);
    ui.frame();
    ui.pointer_move(end, PointerKind::Mouse);
    ui.frame();
    if let Some((label, _)) = ui.tree().drag_label() {
        notes.push(format!("arrastou {label:?}"));
    }
    ui.pointer_up(end, PointerKind::Mouse);
    ui.frame();
    Ok(())
}

fn click_at(ui: &mut Ui, point: Point, clicks: u8) {
    ui.pointer_move(point, PointerKind::Mouse);
    ui.frame();
    ui.pointer_down(point, PointerKind::Mouse, clicks);
    ui.frame();
    ui.pointer_up(point, PointerKind::Mouse);
    ui.frame();
}

fn key(ui: &mut Ui, key: Key, mods: Modifiers) {
    ui.set_modifiers(mods);
    ui.key_down(key);
    ui.frame();
    ui.key_up(key);
    ui.set_modifiers(Modifiers::NONE);
    ui.frame();
}

fn parse_key(name: &str) -> Result<Key, String> {
    let lower = name.to_lowercase();
    Ok(match lower.as_str() {
        "enter" | "return" => Key::Enter,
        "space" | " " => Key::Space,
        "tab" => Key::Tab,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "page_up" => Key::PageUp,
        "pagedown" | "page_down" => Key::PageDown,
        _ => {
            let function = lower
                .strip_prefix('f')
                .and_then(|n| n.parse::<u8>().ok())
                .filter(|n| (1..=12).contains(n));
            let mut chars = lower.chars();
            match (function, chars.next(), chars.next()) {
                (Some(n), _, _) => Key::F(n),
                (None, Some(c), None) if c.is_alphanumeric() => Key::Char(c),
                _ => {
                    return Err(format!(
                        "tecla {name:?} desconhecida; use enter, space, tab, escape, backspace, \
                         delete, left, right, up, down, home, end, pageup, pagedown, f1 a f12 \
                         ou uma letra"
                    ));
                }
            }
        }
    })
}

fn parse_modifier(name: &str) -> Result<Modifiers, String> {
    Ok(match name.to_lowercase().as_str() {
        "shift" => Modifiers::SHIFT,
        "ctrl" | "control" => Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        },
        "alt" | "option" => Modifiers {
            alt: true,
            ..Modifiers::NONE
        },
        "meta" | "super" | "win" => Modifiers {
            meta: true,
            ..Modifiers::NONE
        },
        // A tecla dos atalhos de edição: Cmd no macOS, Ctrl no resto.
        "cmd" | "command" => Modifiers::COMMAND,
        _ => {
            return Err(format!(
                "modificador {name:?} desconhecido; use shift, ctrl, alt, meta ou cmd"
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teclas_e_modificadores() {
        assert_eq!(parse_key("Enter"), Ok(Key::Enter));
        assert_eq!(parse_key("A"), Ok(Key::Char('a')));
        assert_eq!(parse_key("F5"), Ok(Key::F(5)));
        assert_eq!(parse_key("f12"), Ok(Key::F(12)));
        assert!(parse_key("F13").is_err());
        assert!(parse_key("F0").is_err());
        assert_eq!(parse_modifier("cmd"), Ok(Modifiers::COMMAND));
        assert!(parse_modifier("hyper").is_err());
    }

    #[test]
    fn pedidos_em_json() {
        let r: Request = serde_json::from_str(
            r#"{"cmd":"act","actions":[{"action":"type","id":"email","text":"a@b"},{"action":"key","key":"tab","modifiers":["shift"]}]}"#,
        )
        .unwrap();
        assert!(matches!(r, Request::Act { actions } if actions.len() == 2));
        let erro = serde_json::from_str::<Request>(r#"{"cmd":"clik"}"#).unwrap_err();
        assert!(erro.to_string().contains("unknown variant"), "{erro}");
    }

    #[test]
    fn a_grade_de_propriedades_serializa_o_hidden() {
        use crate::prelude::*;
        use tomni_ui_theme::{FluentDrawer, Theme};

        let mut ui = Ui::new(390, 400, FluentDrawer::new(Theme::Light), || {
            let host = signal("a".to_owned());
            let proxy = signal("b".to_owned());
            Form::new("Conexão").child(
                PropertyGrid::new()
                    .id("props")
                    .category("Conexão")
                    .text("Host", host)
                    .category("Proxy")
                    .hidden(true)
                    .text("Endereço", proxy),
            )
        });
        ui.frame();
        fn achar(no: &serde_json::Value, id: &str) -> Option<serde_json::Value> {
            if no["id"] == id {
                return Some(no.clone());
            }
            no["children"]
                .as_array()?
                .iter()
                .find_map(|filho| achar(filho, id))
        }
        let json = serde_json::to_value(ui.inspect()).unwrap();
        let achada = achar(&json, "props").expect("a grade na árvore");
        assert_eq!(achada["properties"][1]["hidden"], true);
        assert_eq!(achada["categories"][1]["hidden"], true);
        assert_eq!(achada["categories"][0]["hidden"], false);
    }

    #[test]
    fn select_e_set_value_nos_widgets_novos() {
        use crate::prelude::*;
        use tomni_ui_theme::{FluentDrawer, Theme};

        let mut ui = Ui::new(390, 600, FluentDrawer::new(Theme::Light), || {
            let tema = signal("Claro".to_owned());
            let volume = signal(0.0);
            let aba = signal("geral".to_owned());
            let quantidade = signal(1.0);
            Form::new("Ajustes").child(
                Column::new()
                    .child(RadioButtons::bind(tema, ["Claro", "Escuro"]).id("tema"))
                    .child(Slider::bind(volume, 0..=100).step(10.0).id("volume"))
                    .child(
                        TabView::bind(aba)
                            .id("config")
                            .height(160)
                            .tab("geral", "Geral", Text::new("Um"))
                            .tab("avancado", "Avançado", Text::new("Dois")),
                    )
                    .child(NumberBox::bind(quantidade).id("quantidade").precision(0)),
            )
        });
        ui.frame();
        let mut notes = Vec::new();
        let actions: Vec<Action> = serde_json::from_str(
            r#"[{"action":"select","id":"tema","option":"Escuro"},
                {"action":"select","id":"config","option":"Avançado"},
                {"action":"set_value","id":"volume","value":72},
                {"action":"set_value","id":"quantidade","value":7.4}]"#,
        )
        .unwrap();
        for action in actions {
            act(&mut ui, action, &mut notes).unwrap();
        }
        let text = |ui: &Ui, id: &str| untrack(|| ui.tree().widget(ui.find(id).unwrap()).text());
        assert_eq!(text(&ui, "tema").as_deref(), Some("Escuro"));
        assert_eq!(text(&ui, "config").as_deref(), Some("avancado"));
        assert_eq!(text(&ui, "volume").as_deref(), Some("70"));
        assert_eq!(text(&ui, "quantidade").as_deref(), Some("7"));
        assert_eq!(notes, ["volume ficou em 70", "quantidade ficou em 7"]);

        let erro = select(&mut ui, "tema", "Roxo", &mut notes).unwrap_err();
        assert_eq!(erro, "tema não tem a opção \"Roxo\"; opções: Claro, Escuro");
        let erro = act(
            &mut ui,
            Action::SetValue {
                id: "tema".into(),
                value: 1.0,
            },
            &mut notes,
        )
        .unwrap_err();
        assert_eq!(erro, "tema não é um `Slider` nem um `NumberBox`");
    }

    #[test]
    fn clique_do_meio_fecha_a_aba() {
        use crate::prelude::*;
        use tomni_ui_theme::{FluentDrawer, Theme};

        let abertas = signal(vec!["Um".to_owned(), "Dois".to_owned()]);
        let mut ui = Ui::new(390, 300, FluentDrawer::new(Theme::Light), move || {
            let aba = signal("um".to_owned());
            Form::new("Abas").child(
                TabView::bind(aba)
                    .id("abas")
                    .grow(1)
                    .items(abertas, |t| t.to_lowercase(), |t| t.clone(), Text::new)
                    .on_close(move |chave| {
                        abertas.update(|v| v.retain(|t| t.to_lowercase() != chave))
                    }),
            )
        });
        ui.frame();
        let mut notes = Vec::new();
        let action: Action =
            serde_json::from_str(r#"{"action":"middle_click","id":"abas-dois"}"#).unwrap();
        act(&mut ui, action, &mut notes).unwrap();
        assert_eq!(abertas.get_untracked(), ["Um"]);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn arrastar_a_aba_e_abrir_o_menu_de_contexto() {
        use crate::prelude::*;
        use tomni_ui_theme::{FluentDrawer, Theme};

        let abertas = signal(vec!["Um".to_owned(), "Dois".to_owned(), "Três".to_owned()]);
        let mut ui = Ui::new(600, 300, FluentDrawer::new(Theme::Light), move || {
            let aba = signal("um".to_owned());
            Form::new("Abas").child(
                Column::new()
                    .child(
                        TabView::bind(aba)
                            .id("abas")
                            .items(abertas, |t| t.to_lowercase(), |t| t.clone(), Text::new)
                            .on_reorder(move |de, para| {
                                abertas.update(|v| {
                                    let t = v.remove(de);
                                    v.insert(para, t);
                                })
                            }),
                    )
                    .child(Text::new("linha").id("linha").context_flyout(
                        MenuFlyout::new().id("menu").item("copiar", "Copiar", || {}),
                    )),
            )
        });
        ui.frame();
        let mut notes = Vec::new();
        let action: Action =
            serde_json::from_str(r#"{"action":"drag","id":"abas-três","to":"abas-um"}"#).unwrap();
        act(&mut ui, action, &mut notes).unwrap();
        assert_eq!(abertas.get_untracked(), ["Três", "Um", "Dois"]);
        assert_eq!(notes, [r#"arrastou "Três""#]);

        let action: Action = serde_json::from_str(r#"{"action":"drag","id":"abas-um"}"#).unwrap();
        let erro = act(&mut ui, action, &mut notes).unwrap_err();
        assert!(erro.contains("`to`"), "{erro}");

        notes.clear();
        let action: Action = serde_json::from_str(r#"{"action":"context","id":"linha"}"#).unwrap();
        act(&mut ui, action, &mut notes).unwrap();
        assert_eq!(notes, ["menu menu aberto"]);
        assert!(ui.inspect().find("linha").unwrap().context_flyout);

        let action: Action = serde_json::from_str(r#"{"action":"context","id":"abas"}"#).unwrap();
        let erro = act(&mut ui, action, &mut notes).unwrap_err();
        assert!(erro.contains("não tem menu de contexto"), "{erro}");
    }

    #[test]
    fn ids_com_acento_chegam_inteiros() {
        use crate::prelude::*;
        use tomni_ui_theme::{FluentDrawer, Theme};

        let tela = || {
            let cidade = signal(String::new());
            Form::new("Cidade").child(
                Column::new().child(
                    AutoSuggestBox::bind(cidade, |texto: &str| {
                        ["Belo Horizonte", "Belém", "São Paulo"]
                            .into_iter()
                            .filter(|c| !texto.is_empty() && c.starts_with(texto))
                            .map(str::to_owned)
                            .collect()
                    })
                    .id("cidade"),
                ),
            )
        };
        // O id derivado guarda o acento; o JSON chega em UTF-8 ou com escape.
        for json in [
            r#"[{"action":"type","id":"cidade","text":"Bel"},{"action":"click","id":"cidade-belém"}]"#,
            r#"[{"action":"type","id":"cidade","text":"Bel"},{"action":"click","id":"cidade-bel\u00e9m"}]"#,
        ] {
            let mut ui = Ui::new(390, 600, FluentDrawer::new(Theme::Light), tela);
            ui.frame();
            let actions: Vec<Action> = serde_json::from_str(json).unwrap();
            let mut notes = Vec::new();
            for action in actions {
                act(&mut ui, action, &mut notes).unwrap();
            }
            let text = untrack(|| ui.tree().widget(ui.find("cidade").unwrap()).text());
            assert_eq!(text.as_deref(), Some("Belém"));
        }

        let mut ui = Ui::new(390, 600, FluentDrawer::new(Theme::Light), tela);
        ui.frame();
        let mut notes = Vec::new();
        let action = Action::Type {
            id: Some("cidade".into()),
            text: "Bel".into(),
        };
        act(&mut ui, action, &mut notes).unwrap();
        let erro = node(&ui, "cidade-belem").unwrap_err();
        assert!(
            erro.contains("cidade-belém"),
            "a lista de ids mostra o acento: {erro}"
        );
    }

    /// O `type` num `TerminalPanes` vai ao terminal do painel ativo e segue o envio para vários.
    #[cfg(feature = "terminal")]
    #[test]
    fn type_num_terminal_panes() {
        use crate::prelude::*;
        use crate::terminal::{Broadcast, Loopback, PaneLayout, Session, TerminalPanes};
        use tomni_ui_theme::{FluentDrawer, Theme};

        let loopbacks: Vec<Loopback> = (0..2).map(|_| Loopback::new()).collect();
        let handles: Vec<_> = loopbacks.iter().map(Loopback::handle).collect();
        let sessoes: Vec<Session> = loopbacks
            .into_iter()
            .enumerate()
            .map(|(i, l)| Session::new(l).key(format!("s{i}")).name(format!("s{i}")))
            .collect();
        let envio = std::rc::Rc::new(std::cell::Cell::new(None));
        let guarda = std::rc::Rc::clone(&envio);
        let mut ui = Ui::new(900, 600, FluentDrawer::new(Theme::Light), move || {
            let layout = signal(PaneLayout::with(sessoes.clone()));
            layout.update(|l| l.grid(2, 1));
            let broadcast = signal(Broadcast::None);
            guarda.set(Some(broadcast));
            Form::new("Painéis").child(
                TerminalPanes::bind(layout)
                    .broadcast(broadcast)
                    .id("paineis")
                    .grow(1),
            )
        });
        ui.frame();
        let digitar = |ui: &mut Ui| {
            let mut notes = Vec::new();
            let action = Action::Type {
                id: Some("paineis".into()),
                text: "ls\n".into(),
            };
            act(ui, action, &mut notes).unwrap();
        };
        let escrito = |h: &tomni_ui_terminal::LoopbackHandle| {
            String::from_utf8_lossy(&h.take_written()).into_owned()
        };

        // Sem envio, só o painel ativo (o primeiro) recebe.
        digitar(&mut ui);
        assert_eq!(escrito(&handles[0]), "ls\r");
        assert_eq!(escrito(&handles[1]), "");

        // Com o envio para todos, as duas sessões recebem.
        envio.get().unwrap().set(Broadcast::All);
        ui.frame();
        digitar(&mut ui);
        assert_eq!(escrito(&handles[0]), "ls\r");
        assert_eq!(escrito(&handles[1]), "ls\r");
    }
}
