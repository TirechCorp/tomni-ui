//! Lint da tela montada: contraste AA (do texto e dos ícones), alvo de toque, texto cortado,
//! caracteres sem glifo, imagens sem texto alternativo e widgets fora da janela (ou da área rolável do `ScrollViewer` em volta). Widgets ocultos
//! (`.visible(false)`) e popups fechados ficam de fora; os rolados para fora da área visível,
//! não. Um popup aberto é conferido contra o fundo dele e a janela.
//! É o `tomni-ui lint` e o `ui_lint` do MCP; nos testes, `TestApp::assert_lint`.

use std::collections::{HashMap, HashSet};
use std::fmt;

use tomni_ui_core::kurbo::Rect;
use tomni_ui_core::{Density, Look, NodeId, Shortcut};
use tomni_ui_reactive::untrack;
use tomni_ui_render::peniko::Color;
use tomni_ui_theme::over;

use crate::Ui;

/// Menor lado aceito para um widget interativo, em pixels lógicos (alvo de toque), na
/// densidade padrão. Na compacta, é [`MIN_TARGET_COMPACT`].
pub const MIN_TARGET: f64 = 44.0;

/// Menor lado aceito para um widget interativo na densidade compacta
/// (`.density(Density::Compact)`): a altura dos campos do `Compact.xaml` do WinUI.
pub const MIN_TARGET_COMPACT: f64 = 24.0;

/// Contraste mínimo do nível AA (WCAG 2.2) para texto comum.
pub const MIN_CONTRAST: f64 = 4.5;

/// Contraste mínimo do nível AA para texto grande (24 px, ou 18,66 px em negrito).
pub const MIN_CONTRAST_LARGE: f64 = 3.0;

/// Contraste mínimo de um ícone que passa informação contra o fundo (WCAG 1.4.11).
pub const MIN_CONTRAST_GRAPHIC: f64 = 3.0;

/// Regras do lint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "tools", derive(serde::Serialize))]
#[cfg_attr(feature = "tools", serde(rename_all = "kebab-case"))]
pub enum Rule {
    /// Texto com contraste abaixo do AA contra o fundo (o da janela ou do popup, com os fundos
    /// dos `Border` e `Card` em volta por cima). Widgets desabilitados, ou dentro de um
    /// desabilitado, não contam, como na WCAG. Vale também para os ícones (`Icon` e botão só
    /// de ícone), com o mínimo de [`MIN_CONTRAST_GRAPHIC`], exceto os `.decorative()`.
    Contrast,
    /// Widget interativo com largura ou altura abaixo de [`MIN_TARGET`]. Os links de um
    /// `RichText` não contam: estão no meio do texto corrido (exceção "em linha" da WCAG 2.2).
    TouchTarget,
    /// Texto que não cabe no widget e sai cortado.
    ClippedText,
    /// Widget fora da janela, no todo ou em parte. Dentro de um `ScrollViewer`, fora da área
    /// que dá para alcançar rolando: o conteúdo inteiro no eixo que rola e a área visível no
    /// outro.
    Offscreen,
    /// Widget interativo sem `.id()`: testes, CLI e MCP não conseguem encontrá-lo.
    MissingId,
    /// Caractere que as fontes embutidas não têm: aparece como um retângulo.
    MissingGlyph,
    /// `Card` clicável ou botão só de ícone sem `.label`: a árvore, os testes e o leitor de
    /// tela não têm um nome para ele.
    MissingLabel,
    /// `Image` sem `.alt("...")` nem `.decorative()`.
    MissingAlt,
    /// `WebView` dentro de um `ScrollViewer`: a página é uma janela do sistema, que rola por
    /// conta própria e não acompanha a rolagem nem o corte da tela.
    WebViewInScroll,
    /// Dois `.accelerator()` (ou `.shortcut()` de item de menu, ou `.access_key()` de menu da
    /// barra) com o mesmo atalho valendo ao mesmo tempo: só o primeiro, na ordem da árvore,
    /// roda.
    DuplicateAccelerator,
}

impl Rule {
    /// Todas as regras.
    pub const ALL: [Self; 10] = [
        Self::Contrast,
        Self::TouchTarget,
        Self::ClippedText,
        Self::Offscreen,
        Self::MissingId,
        Self::MissingGlyph,
        Self::MissingLabel,
        Self::MissingAlt,
        Self::WebViewInScroll,
        Self::DuplicateAccelerator,
    ];

    /// Nome usado na CLI e no MCP (`touch-target`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Contrast => "contrast",
            Self::TouchTarget => "touch-target",
            Self::ClippedText => "clipped-text",
            Self::Offscreen => "offscreen",
            Self::MissingId => "missing-id",
            Self::MissingGlyph => "missing-glyph",
            Self::MissingLabel => "missing-label",
            Self::MissingAlt => "missing-alt",
            Self::WebViewInScroll => "webview-in-scroll",
            Self::DuplicateAccelerator => "duplicate-accelerator",
        }
    }

    /// Regra com o nome `name`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|rule| rule.name() == name)
    }

    /// Gravidade: [`Rule::MissingId`] e [`Rule::MissingLabel`] são avisos.
    pub fn severity(self) -> Severity {
        match self {
            Self::MissingId | Self::MissingLabel => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Gravidade de um problema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "tools", derive(serde::Serialize))]
#[cfg_attr(feature = "tools", serde(rename_all = "lowercase"))]
pub enum Severity {
    /// Precisa ser corrigido: a tela não passa no lint.
    Error,
    /// Recomendação.
    Warning,
}

/// Um problema encontrado.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "tools", derive(serde::Serialize))]
pub struct Issue {
    /// Regra.
    pub rule: Rule,
    /// Gravidade.
    pub severity: Severity,
    /// Id do widget ou, sem id, o tipo (`"button"`).
    pub widget: String,
    /// O que está errado e como corrigir.
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "erro",
            Severity::Warning => "aviso",
        };
        write!(
            f,
            "{severity}[{}] {}: {}",
            self.rule, self.widget, self.message
        )
    }
}

/// Razão de contraste da WCAG entre duas cores opacas, de 1 a 21.
pub fn contrast_ratio(a: Color, b: Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (light, dark) = if la > lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}

/// Luminância relativa da WCAG.
fn luminance(color: Color) -> f64 {
    let [r, g, b, _] = color.components.map(|c| {
        let c = f64::from(c);
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn hex(color: Color) -> String {
    let c = color.to_rgba8();
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// Duas casas decimais, com vírgula.
fn decimal(value: f64) -> String {
    format!("{value:.2}").replace('.', ",")
}

impl Ui {
    /// Confere a tela atual. Os erros vêm antes dos avisos; dentro de cada grupo, na ordem da
    /// árvore.
    ///
    /// ```
    /// use tomni_ui::lint::Rule;
    /// use tomni_ui::prelude::*;
    /// use tomni_ui::theme::{FluentDrawer, Theme};
    /// use tomni_ui::Ui;
    ///
    /// let mut ui = Ui::new(320, 200, FluentDrawer::new(Theme::Light), || {
    ///     Form::new("Olá").child(Column::new().child(Button::new("OK").id("ok")))
    /// });
    /// let issues = ui.lint();
    /// // 32 px de altura, o padrão do Fluent, é pouco para o dedo.
    /// assert_eq!(issues[0].rule, Rule::TouchTarget);
    /// ```
    pub fn lint(&mut self) -> Vec<Issue> {
        let (width, height) = self.size();
        let window = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let mut issues = Vec::new();
        // Grupos sem id já apontados por uma das partes que montaram.
        let mut groups = HashSet::new();
        let ids: Vec<NodeId> = self.tree().ids().collect();
        for id in ids {
            self.lint_widget(id, window, &mut issues, &mut groups);
        }
        self.lint_accelerators(&mut issues);
        issues.sort_by_key(|issue| issue.severity);
        issues
    }

    /// Atalhos repetidos entre os que valem agora.
    fn lint_accelerators(&self, issues: &mut Vec<Issue>) {
        let tree = self.tree();
        let mut first: HashMap<Shortcut, NodeId> = HashMap::new();
        for id in tree.ids() {
            let shortcuts: Vec<Shortcut> = tree
                .widget(id)
                .common()
                .accelerators
                .iter()
                .flat_map(|a| a.shortcuts())
                .filter(|s| !s.is_none())
                .collect();
            if shortcuts.is_empty() || !tree.accelerator_active(id) {
                continue;
            }
            for shortcut in shortcuts {
                let Some(&owner) = first.get(&shortcut) else {
                    first.insert(shortcut, id);
                    continue;
                };
                let message = if owner == id {
                    format!("o atalho {shortcut} aparece duas vezes no mesmo widget: tire um")
                } else {
                    format!(
                        "o atalho {shortcut} já é de {}, que vale ao mesmo tempo e roda primeiro: troque um dos dois",
                        tree.name(owner)
                    )
                };
                issues.push(Issue {
                    rule: Rule::DuplicateAccelerator,
                    severity: Rule::DuplicateAccelerator.severity(),
                    widget: tree.name(id),
                    message,
                });
            }
        }
    }

    fn lint_widget(
        &mut self,
        id: NodeId,
        window: Rect,
        issues: &mut Vec<Issue>,
        groups: &mut HashSet<NodeId>,
    ) {
        if !self.shown(id) {
            return;
        }
        let tree = self.tree();
        let name = tree.name(id);
        let rect = tree.rect(id);
        let target = tree.hit_rect(id);
        let widget = tree.widget(id);
        let scroller = widget.scroller().is_some();
        // A barra de rolagem é fina como a do WinUI: no toque, rola-se arrastando o conteúdo.
        let interactive = widget.interactive() && !scroller;
        let named = widget.interactive();
        let leaf = widget.flex().is_none() && !widget.fragment() || scroller;
        let enabled = untrack(|| widget.enabled());
        let label = untrack(|| widget.text()).filter(|label| !label.trim().is_empty());
        let has_id = widget.common().id.is_some();
        let generated = widget.common().is_generated();
        let kind = widget.kind();
        let decorative = widget.decorative();
        drop(widget);
        let look = self.look(id);
        let ancestors: Vec<NodeId> = std::iter::successors(tree.parent(id), |&p| tree.parent(p))
            .take_while(|&p| tree.layer(p).is_none())
            .collect();
        let enabled = enabled
            && ancestors
                .iter()
                .all(|&p| untrack(|| tree.widget(p).enabled()));
        let (area, inside) = reachable(self, id, window);
        if named && !has_id && generated {
            // Uma opção ou aba sem id: falta o id no grupo, que o app escreveu. Um grupo que
            // recebe o ponteiro (`ComboBox`) já aparece por conta própria.
            let group = std::iter::successors(tree.parent(id), |&p| tree.parent(p))
                .find(|&p| tree.widget(p).names_children());
            if let Some(group) = group
                && !tree.widget(group).interactive()
                && groups.insert(group)
            {
                let kind = tree.widget(group).kind();
                issues.push(Issue {
                    rule: Rule::MissingId,
                    severity: Rule::MissingId.severity(),
                    widget: tree.name(group),
                    message: format!(
                        "{kind} sem id: use `.id(\"...\")` para testes, CLI e MCP; as partes dele ganham ids derivados (`<id>-<parte>`)"
                    ),
                });
            }
        }
        let mut issue = |rule: Rule, message: String| {
            issues.push(Issue {
                rule,
                severity: rule.severity(),
                widget: name.clone(),
                message,
            });
        };

        if leaf && !area.inflate(0.5, 0.5).contains_rect(rect) {
            let message = match inside {
                None => format!(
                    "ocupa ({}, {}) a ({}, {}), fora da janela de {}×{}: diminua o conteúdo ou o espaçamento, ou ponha a tela num `ScrollViewer`",
                    rect.x0,
                    rect.y0,
                    rect.x1,
                    rect.y1,
                    window.width(),
                    window.height(),
                ),
                Some(sv) => format!(
                    "ocupa ({}, {}) a ({}, {}), fora da área de ({}, {}) a ({}, {}) que o `ScrollViewer` {sv} alcança rolando: diminua o conteúdo no eixo que não rola ou use `ScrollViewer::both()`",
                    rect.x0, rect.y0, rect.x1, rect.y1, area.x0, area.y0, area.x1, area.y1,
                ),
            };
            issue(Rule::Offscreen, message);
        }

        // A barra de título segue as medidas do WinUI (32 ou 48 px de altura), feitas para o
        // mouse: no Android, onde há toque, a barra do sistema fica por cima dela. A barra de
        // menus também (itens de 32 px), como no Windows, e o divisor do `Split` (8 px, que o
        // teclado move sem arrastar). O divisor da descrição do `PropertyGrid` também (8 px, só
        // de ponteiro). As linhas e o cabeçalho do `DataGrid` também (32 px na densidade
        // padrão, como a lista do WinUI; a tela de toque usa a compacta ou outra lista). E os
        // links de um `RichText`: estão no meio do texto corrido, a exceção "em linha" do
        // critério 2.5.8 da WCAG 2.2 (M47).
        let title_bar = kind == "rich_text"
            || kind == "title_bar"
            || kind == "data_grid_row"
            || kind == "data_grid_header"
            || kind == "menu_bar_item"
            || kind == "split_sizer"
            || kind == "property_grid_sizer"
            || ancestors
                .iter()
                .any(|&p| tree.widget(p).kind() == "title_bar");
        let density = tree.density(id);
        let min = density.min_target();
        if interactive && !title_bar && (target.width() < min || target.height() < min) {
            let fix = match (target.width() < min, target.height() < min) {
                (true, true) => format!(".width({min}).height({min})"),
                (true, false) => format!(".width({min})"),
                _ => format!(".height({min})"),
            };
            let where_ = match density {
                Density::Standard => "na densidade padrão",
                Density::Compact => "na densidade compacta",
            };
            issue(
                Rule::TouchTarget,
                format!(
                    "{kind} de {}×{} px; alvos de toque precisam de pelo menos {min}×{min} {where_}: use `{fix}`",
                    target.width(),
                    target.height(),
                ),
            );
        }

        if named && !has_id && !generated {
            issue(
                Rule::MissingId,
                format!("{kind} sem id: use `.id(\"...\")` para testes, CLI e MCP"),
            );
        }

        if kind == "card" && named && label.is_none() {
            issue(
                Rule::MissingLabel,
                "card clicável sem rótulo: use `.label(\"...\")` com o que ele abre, para a árvore, os testes e o leitor de tela".to_owned(),
            );
        }

        let unnamed = match &look {
            Look::Button { label, visual } => visual.icon_only && label.trim().is_empty(),
            Look::ToggleButton { label, visual } => visual.icon_only && label.trim().is_empty(),
            _ => false,
        };
        if unnamed {
            issue(
                Rule::MissingLabel,
                format!(
                    "{kind} só de ícone sem rótulo: use `.label(\"...\")` com o que ele faz; vira a dica e o nome na árvore, nos testes e no leitor de tela"
                ),
            );
        }

        if kind == "image" && label.is_none() && !decorative {
            issue(
                Rule::MissingAlt,
                "imagem sem texto alternativo: use `.alt(\"...\")` com o que ela mostra ou, se for só enfeite, `.decorative()`".to_owned(),
            );
        }

        if kind == "webview"
            && let Some(sv) = ancestors
                .iter()
                .find(|&&p| tree.widget(p).scroller().is_some())
        {
            issue(
                Rule::WebViewInScroll,
                format!(
                    "webview dentro do `ScrollViewer` {}: a página rola sozinha; tire o `ScrollViewer` e dê ao `WebView` o espaço que sobra com `.grow(1.0)`",
                    tree.name(*sv)
                ),
            );
        }

        // Dentro de um popup, o texto fica sobre o fundo dele, não sobre o do `Form`; por cima,
        // os fundos dos `Border` e `Card` em volta, de fora para dentro, no centro do widget.
        // Uma sobreposição com fundo próprio (o crachá de um painel) conta como um `Border`.
        let surface = tree.layer_of(id).and_then(|layer| {
            let look = tree.look(layer);
            self.drawer().surface(&look).or_else(|| {
                self.drawer()
                    .background(tree.rect(layer), &look, rect.center())
            })
        });
        let center = rect.center();
        let mut backdrop = surface;
        for &ancestor in ancestors.iter().rev() {
            let look = tree.look(ancestor);
            if let Some(color) = self.drawer().background(tree.rect(ancestor), &look, center) {
                let base = backdrop.unwrap_or(self.drawer().tokens().solid_background_base);
                backdrop = Some(over(color, base));
            }
        }
        self.drawer_mut().set_backdrop(backdrop);
        let texts = self.drawer_mut().text_info(rect, &look);
        let graphics = self.drawer_mut().graphic_info(rect, &look);
        self.drawer_mut().set_backdrop(None);
        for graphic in graphics.into_iter().filter(|_| enabled) {
            let ratio = contrast_ratio(graphic.color, graphic.background);
            if ratio < MIN_CONTRAST_GRAPHIC {
                issue(
                    Rule::Contrast,
                    format!(
                        "contraste {}:1 entre o ícone {} ({}) e o fundo {}; o mínimo para ícones é {}:1 (use outra `.color`, ou `.decorative()` se ele só enfeita)",
                        decimal(ratio),
                        graphic.name,
                        hex(graphic.color),
                        hex(graphic.background),
                        decimal(MIN_CONTRAST_GRAPHIC),
                    ),
                );
            }
        }
        for text in texts {
            if enabled {
                let ratio = contrast_ratio(text.color, text.background);
                let large = text.style.size >= 24.0
                    || text.style.size >= 18.66 && text.style.weight >= 700.0;
                let min = if large {
                    MIN_CONTRAST_LARGE
                } else {
                    MIN_CONTRAST
                };
                // Sem arredondar: a WCAG não aceita 4,49 como 4,5.
                if ratio < min {
                    issue(
                        Rule::Contrast,
                        format!(
                            "contraste {}:1 entre o texto {} e o fundo {} em {:?}; o mínimo AA é {}:1",
                            decimal(ratio),
                            hex(text.color),
                            hex(text.background),
                            text.text,
                            decimal(min),
                        ),
                    );
                }
            }
            if let Some(clip) = text.clip
                && !clip.inflate(0.5, 0.5).contains_rect(text.bounds)
            {
                let fix = if text.bounds.width() > clip.width() + 0.5 {
                    "aumente a largura do widget, use um texto mais curto ou reduza o espaçamento em volta"
                } else {
                    "aumente a altura do widget"
                };
                issue(
                    Rule::ClippedText,
                    format!(
                        "{:?} precisa de {}×{} px e só há {}×{}: {fix}",
                        text.text,
                        text.bounds.width().ceil(),
                        text.bounds.height().ceil(),
                        clip.width().max(0.0).floor(),
                        clip.height().max(0.0).floor(),
                    ),
                );
            }
            let missing = self.drawer_mut().missing_glyphs(&text.text, text.style);
            if !missing.is_empty() {
                issue(
                    Rule::MissingGlyph,
                    missing_glyph_message(&text.text, &missing),
                );
            }
        }
    }
}

/// Área onde `id` precisa caber e o nome do `ScrollViewer` que a define: a área que o
/// `ScrollViewer` mais interno em volta alcança rolando ou, fora de um, a janela (um popup
/// abre dentro dela).
fn reachable(ui: &Ui, id: NodeId, window: Rect) -> (Rect, Option<String>) {
    let tree = ui.tree();
    let mut node = (tree.layer(id).is_none())
        .then(|| tree.parent(id))
        .flatten();
    while let Some(parent) = node {
        if let Some(scroller) = tree.scroller(parent) {
            let view = tree.rect(parent);
            let origin = view.origin() - scroller.offset();
            let content = scroller.content();
            let width = if scroller.mode().x() {
                content.width.max(view.width())
            } else {
                view.width()
            };
            let height = if scroller.mode().y() {
                content.height.max(view.height())
            } else {
                view.height()
            };
            let area = Rect::from_origin_size(origin, (width, height));
            return (area, Some(tree.name(parent)));
        }
        if tree.layer(parent).is_some() {
            break;
        }
        node = tree.parent(parent);
    }
    (window, None)
}

/// Lista os caracteres com o código (`"−" (U+2212)`) e sugere a troca.
fn missing_glyph_message(text: &str, missing: &[char]) -> String {
    let chars: Vec<String> = missing
        .iter()
        .map(|&c| format!("\"{c}\" (U+{:04X})", u32::from(c)))
        .collect();
    let (noun, verb) = if missing.len() == 1 {
        ("o caractere", "aparece")
    } else {
        ("os caracteres", "aparecem")
    };
    format!(
        "{text:?} usa {noun} {}, que as fontes embutidas não têm e {verb} como um retângulo: troque por um equivalente (\"-\" no lugar de \"−\", texto no lugar de emoji) ou, para chinês, japonês e coreano, ligue a feature `cjk` do tomni-ui",
        chars.join(", "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contraste_da_wcag() {
        let preto = Color::from_rgb8(0, 0, 0);
        let branco = Color::from_rgb8(255, 255, 255);
        assert!((contrast_ratio(preto, branco) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(branco, branco) - 1.0).abs() < 1e-9);
        // #767676 sobre branco é o cinza mais claro que passa no AA.
        let cinza = Color::from_rgb8(0x76, 0x76, 0x76);
        assert!(contrast_ratio(cinza, branco) >= 4.5);
        let cinza = Color::from_rgb8(0x77, 0x77, 0x77);
        assert!(contrast_ratio(cinza, branco) < 4.5);
    }

    #[test]
    fn mensagem_do_glifo_que_falta() {
        let message = missing_glyph_message("Saldo: \u{2212}10", &['\u{2212}']);
        assert!(message.contains("\"\u{2212}\" (U+2212)"), "{message}");
        assert!(message.contains("o caractere"), "{message}");
        let message = missing_glyph_message("🎉 1º", &['🎉', 'º']);
        assert!(message.contains("(U+1F389), \"º\" (U+00BA)"), "{message}");
    }

    #[test]
    fn nomes_das_regras() {
        for rule in Rule::ALL {
            assert_eq!(Rule::from_name(rule.name()), Some(rule));
        }
    }
}
