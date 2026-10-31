//! Árvore de widgets vista de fora, com id, tipo, retângulo e estado: o que o `tomni-ui tree`
//! e o `ui_tree` do MCP mostram.

use tomni_ui_core::{
    Brush, ButtonState, ButtonVariant, CARD_CORNER_RADIUS, Density, EditState, FieldGlyph,
    InputPurpose, Look, MenuItemKind, NodeId, SETTINGS_CARD_CORNER_RADIUS, ScrollBarState, Symbol,
    parse_number,
};

use tomni_ui_reactive::untrack;

use crate::Ui;

/// Um widget e os filhos.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "tools", derive(serde::Serialize))]
pub struct Node {
    /// Id dado com `.id()`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub id: Option<String>,
    /// Tipo: `form`, `column`, `row`, `uniform_grid`, `border`, `card`, `text`, `rich_text` (com um filho `link`, de id `<id>-link-<n>`, por link), `button`,
    /// `toggle_button`, `edit`, `checkbox`, `toggle_switch`, `info_bar`, `scroll_viewer`, `show`,
    /// `for`, `combo_box`, `content_dialog`, `flyout`, `menu_flyout`, `menu_item`,
    /// `toggle_menu_item`, `radio_menu_item`, `menu_sub_item`, `menu_separator`, `tooltip`, `radio_buttons`,
    /// `slider`, `progress_bar`, `progress_ring`, `expander`, `tab_view`, `navigation_view`,
    /// `auto_suggest_box`, `number_box`, `icon`, `image`, `split`, `menu_bar`, `terminal`,
    /// `webview` e, dentro deles, `combo_box_dropdown`, `combo_box_item`,
    /// `radio_button`, `tab`, `tab_button` (fechar, "+" e as setas da faixa de abas),
    /// `auto_suggest_list`, `auto_suggest_item`, `field_button` (revelar a senha, limpar, aumentar
    /// e diminuir), `number_box_spin` (os botões de incremento sobre o campo), `navigation_pane` (o
    /// painel), `navigation_toggle` (o ☰), `navigation_search` (a lupa do painel compacto),
    /// `navigation_item`, `navigation_flyout` (os filhos de um grupo no painel compacto),
    /// `navigation_dismiss` (a área fora do painel aberto por cima) e `navigation_content` (o fundo
    /// das páginas), `split_pane` (cada painel do `Split`), `split_sizer` (o divisor) e
    /// `menu_bar_item`, `tree_view`, `tree_view_item`, `property_grid`, `property_grid_row` (uma
    /// propriedade), `property_grid_category` (o cabeçalho de uma categoria) e
    /// `property_grid_sizer` (o divisor da descrição).
    pub kind: &'static str,
    /// `[x, y, largura, altura]` na janela, em pixels lógicos. Dentro de um `ScrollViewer`, já
    /// descontada a rolagem: pode cair fora da área visível dele (veja `in_view`).
    pub rect: [f64; 4],
    /// `false` quando o widget está todo fora da área visível de um `ScrollViewer` em volta
    /// (rolado para fora): existe, mas não aparece na tela até rolar.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_true"))]
    pub in_view: bool,
    /// Rolagem `[x, y]` do `ScrollViewer`, em pixels.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub offset: Option<[f64; 2]>,
    /// Tamanho `[largura, altura]` do conteúdo do `ScrollViewer`. Rola enquanto for maior que
    /// o `rect`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub content: Option<[f64; 2]>,
    /// Estado visual: `rest`, `hover`, `pressed`, `focused` ou `disabled`. Só nos widgets
    /// interativos (na `InfoBar`, o do botão de fechar; no `ScrollViewer`, o da barra). Na
    /// `TitleBar`, `active` ou `inactive`, com a janela. No `WebView`, a carga: `loading`,
    /// `ready` ou `failed`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub state: Option<&'static str>,
    /// Se está habilitado.
    pub enabled: bool,
    /// Se aparece: `false` com `.visible(false)` nele ou num ancestral. Um widget oculto fica
    /// com retângulo vazio.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_true"))]
    pub visible: bool,
    /// Nos popups (`content_dialog`, `flyout`, `menu_flyout`, `combo_box_dropdown`, `tooltip`,
    /// `auto_suggest_list`, `number_box_spin`): se está aberto; fechado, fica com `visible: false` e retângulo
    /// vazio. No `expander`: se o conteúdo está à mostra. No `navigation_view`: se o painel
    /// está aberto (largo, com os rótulos); no `navigation_item` de um grupo, se os filhos
    /// estão à mostra.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub open: Option<bool>,
    /// Se tem o foco.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_false"))]
    pub focused: bool,
    /// Título, no `Form`, na `InfoBar`, no `ContentDialog`, no `Terminal` (o que o programa
    /// pôs com `OSC 0` ou `OSC 2`) e no `WebView` (o `<title>` da página).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub title: Option<String>,
    /// Rótulo, no botão, no card clicável, no `ToggleButton`, na `CheckBox`, no `ToggleSwitch` (o de cima), no item de menu, no
    /// item do `ComboBox`, no `RadioButtons` (o cabeçalho) e em cada opção dele, no `Slider`
    /// (o de cima), no cabeçalho do `Expander`, na aba e nos botões dentro de um campo
    /// (`Mostrar senha`, `Limpar`, `Aumentar`, `Diminuir`), e nos itens, no ☰ e na lupa do
    /// `NavigationView`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub label: Option<String>,
    /// Variante do botão (`standard`, `accent` ou `hyperlink`), degrau da rampa do `Text`
    /// (`caption`, `body`, `body_strong`, `body_large`, `subtitle`, `title`, `title_large` ou
    /// `display`), o `Stretch` da `Image` (`none`, `fill`, `uniform` ou `uniform_to_fill`) ou o
    /// modo do `NavigationView` (`expanded`, `compact` ou `minimal`) ou o esquema de cores do
    /// `Terminal` (`campbell`, `theme` ou `custom`) ou a orientação do `Split` (`row`, lado a
    /// lado, ou `column`, um em cima do outro).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub variant: Option<&'static str>,
    /// Texto do campo, como aparece na tela (mascarado no modo senha), o conteúdo do `Text`
    /// (inteiro, mesmo cortado com `max_lines`), o
    /// valor escolhido no `ComboBox` ou no `RadioButtons` (vazio se nenhum) ou a chave da aba
    /// escolhida no `TabView` e no `NavigationView`. No `Terminal`, as linhas da tela, sem os
    /// espaços do fim.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub text: Option<String>,
    /// No `Text` com `max_lines`: se o texto não coube e foi cortado com reticências.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_false"))]
    pub trimmed: bool,
    /// Fundo do `Border` (o token, `#RRGGBB` ou `linear-gradient(...)`) ou do `Card` (o token
    /// do estado).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub background: Option<String>,
    /// Raio dos cantos do `Border` e do `Card`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub corner_radius: Option<f64>,
    /// Placeholder do campo ou do `ComboBox`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub placeholder: Option<String>,
    /// Campo de senha.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_false"))]
    pub password: bool,
    /// Valor do `Slider` ou do `NumberBox` (ausente com o campo vazio ou um texto que não é
    /// número), o tamanho do primeiro painel do `Split` (e do divisor), em px, ou o andamento da `ProgressBar`/`ProgressRing` de 0 a 1 (ausente no
    /// indeterminado).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub value: Option<f64>,
    /// Faixa do `Slider`: `[mínimo, máximo]`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub range: Option<[f64; 2]>,
    /// Se a `CheckBox` está marcada, o `ToggleButton` ou o `ToggleSwitch` ligado, a opção do
    /// `RadioButtons` ou o item de marcar (`toggle_menu_item`) ou de rádio (`radio_menu_item`) do menu marcado.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub checked: Option<bool>,
    /// No item do `ComboBox`: se é o valor escolhido; na aba, se é a escolhida; no item do
    /// `NavigationView`, se tem o indicador de escolhido (ele ou, com os filhos escondidos, o
    /// grupo do escolhido); no item das
    /// sugestões do `AutoSuggestBox`, se está destacado.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub selected: Option<bool>,
    /// Severidade da `InfoBar`: `informational`, `success`, `warning` ou `error`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub severity: Option<&'static str>,
    /// Mensagem da `InfoBar`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub message: Option<String>,
    /// Ícone do `Icon`, do botão (`Button`, `ToggleButton`) ou do item do `NavigationView`, pelo
    /// nome do `Symbol` (`Home`).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub icon: Option<&'static str>,
    /// Selo depois do rótulo de uma linha do `TreeView`, pelo nome do `Symbol`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub badge: Option<&'static str>,
    /// Selo de estado no canto do ícone de uma linha do `TreeView` (`success`, `caution`,
    /// `critical` ou `neutral`).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub status: Option<&'static str>,
    /// Texto alternativo da `Image`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub alt: Option<String>,
    /// Se o `Icon` ou a `Image` é só enfeite (`.decorative()`).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_false"))]
    pub decorative: bool,
    /// Por que a `Image` não abriu (arquivo que não existe, formato desconhecido): ela fica como
    /// um quadro vazio. No `WebView`, por que a página não carregou.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub error: Option<String>,
    /// O tamanho natural que o app informou à `Image` (`natural_size`), largura e altura em pixels.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub natural_size: Option<[u32; 2]>,
    /// Colunas da grade do `Terminal`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub cols: Option<usize>,
    /// Linhas da grade do `Terminal`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub rows: Option<usize>,
    /// Cursor do `Terminal`: `[coluna, linha]` na tela, contando de 0 (ausente quando o
    /// programa o esconde).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub cursor: Option<[usize; 2]>,
    /// Linhas no histórico do `Terminal` (acima da tela) e quantas a vista está rolada para
    /// cima: `[histórico, rolagem]`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub history: Option<[usize; 2]>,
    /// Endereço da página do `WebView` (as embutidas são `fluent://app/...`).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub url: Option<String>,
    /// No `WebView`: se há página para trás no histórico.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub can_go_back: Option<bool>,
    /// No `WebView`: se há página para a frente no histórico.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub can_go_forward: Option<bool>,
    /// Densidade (`standard` ou `compact`), só onde ela muda em relação ao pai (na raiz,
    /// quando não é a padrão): o contêiner com `.density()`. Os descendentes herdam.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub density: Option<&'static str>,
    /// No `TabView`: `[chave, título]` de cada aba, na ordem.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub tabs: Option<Vec<[String; 2]>>,
    /// No `TabView`: se as abas têm botão de fechar (`.on_close`).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_false"))]
    pub closable: bool,
    /// No `TabView`: se dá para arrastar as abas para mudar a ordem (`.on_reorder`).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_false"))]
    pub reorderable: bool,
    /// No `TerminalPanes`: a forma do layout (`col[row[0,1],2]`, os números são os painéis).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub layout: Option<String>,
    /// No `TerminalPanes`: os painéis, na ordem das folhas do layout.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub panes: Option<Vec<PaneNodeInfo>>,
    /// No `TerminalPanes`: o índice do painel ativo.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub active_pane: Option<usize>,
    /// No `TerminalPanes`: o índice do painel ampliado, se há.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub zoomed: Option<usize>,
    /// No `TerminalPanes`: para quem vai o que se digita (`none`, `visible`, `all` ou
    /// `group:a,b`).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub broadcast: Option<String>,
    /// No `TerminalPanes`: os crachás com o número dos painéis estão ligados (`.identify()`).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_false"))]
    pub identifying: bool,
    /// No `TerminalPanes`: o índice do painel em destaque (`.highlight_pane()`), se há.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub highlighted: Option<usize>,
    /// Atalhos de teclado do widget (`.accelerator()`, ou o `.shortcut()` do item de menu, ou
    /// o Alt e a letra do item da `MenuBar`), como aparecem no menu: `Ctrl+N`, `Ctrl+Shift+F`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Vec::is_empty"))]
    pub accelerators: Vec<String>,
    /// Se o widget tem menu de contexto (`.context_flyout()`, ou o `.tab_context_flyout()` na
    /// aba): o botão direito, o toque longo e o Shift+F10 abrem. O menu é um filho, o
    /// `menu_flyout`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_false"))]
    pub context_flyout: bool,
    /// No `TreeView`: as linhas visíveis, em ordem (no máximo [`MAX_TREE_ITEMS`]).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub items: Option<Vec<TreeItemInfo>>,
    /// No `TreeView`: quantas linhas estão visíveis ao todo (`items` pode ter menos).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub items_total: Option<usize>,
    /// No `TreeView`: o texto do filtro, se há (`.filter()` não vazio).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub filter: Option<String>,
    /// Na `PropertyGrid`: todas as propriedades, na ordem da declaração (a senha mascarada, a
    /// `.check` como `true` ou `false`).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub properties: Option<Vec<PropertyNodeInfo>>,
    /// Na `PropertyGrid`: as categorias, na ordem da declaração.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub categories: Option<Vec<CategoryNodeInfo>>,
    /// Na `PropertyGrid`: se está na ordem alfabética.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub alphabetical: Option<bool>,
    /// Na linha da `PropertyGrid` (`property_grid_row`): se o valor difere do `.default`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "is_false"))]
    pub changed: bool,
    /// No `DataGrid`: os títulos das colunas, na ordem.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub grid_columns: Option<Vec<String>>,
    /// No `DataGrid`: a coluna ordenada e o sentido (`nome crescente`, `tamanho decrescente`).
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub grid_sort: Option<String>,
    /// No `DataGrid`: as linhas visíveis, em ordem (no máximo [`MAX_TREE_ITEMS`]); o total
    /// está em `items_total`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub grid_rows: Option<Vec<GridRowInfo>>,
    /// No `DataGrid`: as chaves selecionadas que aparecem, na ordem das linhas.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub grid_selected: Option<Vec<String>>,
    /// Descrição para quem não vê a cor: na linha do `DataGrid` com tom (`DataGridRow::tone`),
    /// "destaque: sucesso", "destaque: atenção" ou "destaque: crítico".
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub description: Option<String>,
    /// Nível da linha do `TreeView` (`tree_view_item`): 0 na raiz.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub depth: Option<usize>,
    /// Filhos, na ordem da árvore.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Vec::is_empty"))]
    pub children: Vec<Node>,
}

/// Um painel do `TerminalPanes`, como as ferramentas o mostram.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "tools", derive(serde::Serialize))]
pub struct PaneNodeInfo {
    /// `[chave, título]` de cada aba, da esquerda para a direita.
    pub tabs: Vec<[String; 2]>,
    /// A chave da aba à mostra.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub selected: Option<String>,
}

/// Uma propriedade da `PropertyGrid`, como as ferramentas a mostram.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "tools", derive(serde::Serialize))]
pub struct PropertyNodeInfo {
    /// A categoria; `None` antes da primeira.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub category: Option<String>,
    /// O nome.
    pub name: String,
    /// O valor como a linha o mostra: a senha mascarada, a `.check` como `true` ou `false`.
    pub value: String,
    /// Se o valor difere do `.default`.
    pub changed: bool,
    /// Se é a linha escolhida.
    pub selected: bool,
    /// Se é só de leitura.
    pub read_only: bool,
    /// Se está habilitada.
    pub enabled: bool,
    /// Se não aparece (`.hidden` nela, na categoria ou na categoria inteira); o valor fica.
    pub hidden: bool,
}

/// Uma categoria da `PropertyGrid`, como as ferramentas a mostram.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "tools", derive(serde::Serialize))]
pub struct CategoryNodeInfo {
    /// O nome.
    pub name: String,
    /// Se está aberta.
    pub expanded: bool,
    /// Se não aparece: escondida por `.hidden` ou sem nenhuma linha visível.
    pub hidden: bool,
}

/// Quantas linhas do `TreeView` o [`Node::items`] lista.
pub const MAX_TREE_ITEMS: usize = 500;

/// Uma linha visível do `TreeView`, como as ferramentas a mostram.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "tools", derive(serde::Serialize))]
pub struct TreeItemInfo {
    /// Chave do item.
    pub key: String,
    /// Rótulo.
    pub label: String,
    /// Nível: 0 na raiz.
    pub depth: usize,
    /// Se a pasta está aberta.
    pub expanded: bool,
    /// Se é o item selecionado.
    pub selected: bool,
}

/// Uma linha visível do `DataGrid`, como as ferramentas a mostram.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "tools", derive(serde::Serialize))]
pub struct GridRowInfo {
    /// Chave da linha.
    pub key: String,
    /// O texto de cada célula, na ordem das colunas.
    pub cells: Vec<String>,
    /// Se está selecionada.
    pub selected: bool,
    /// O tom do fundo (`DataGridRow::tone`): `"success"`, `"caution"` ou `"critical"`.
    #[cfg_attr(feature = "tools", serde(skip_serializing_if = "Option::is_none"))]
    pub tone: Option<String>,
}

#[cfg(feature = "tools")]
#[allow(clippy::trivially_copy_pass_by_ref, reason = "assinatura do serde")]
fn is_false(value: &bool) -> bool {
    !value
}

#[cfg(feature = "tools")]
#[allow(clippy::trivially_copy_pass_by_ref, reason = "assinatura do serde")]
fn is_true(value: &bool) -> bool {
    *value
}

fn state_name(state: ButtonState) -> &'static str {
    match state {
        ButtonState::Rest => "rest",
        ButtonState::Hover => "hover",
        ButtonState::Pressed => "pressed",
        ButtonState::Disabled => "disabled",
    }
}

impl Node {
    /// Procura o widget com o id `id` neste nó e nos descendentes (os popups também, abertos
    /// ou não).
    pub fn find(&self, id: &str) -> Option<&Node> {
        if self.id.as_deref() == Some(id) {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }
}

impl Ui {
    /// A árvore de widgets agora, com id, tipo, retângulo e estado.
    ///
    /// ```
    /// use tomni_ui::prelude::*;
    /// use tomni_ui::theme::{FluentDrawer, Theme};
    /// use tomni_ui::Ui;
    ///
    /// let ui = Ui::new(320, 200, FluentDrawer::new(Theme::Light), || {
    ///     Form::new("Olá").child(Button::new("OK").id("ok"))
    /// });
    /// let tree = ui.inspect();
    /// assert_eq!(tree.title.as_deref(), Some("Olá"));
    /// assert_eq!(tree.find("ok").unwrap().label.as_deref(), Some("OK"));
    /// ```
    pub fn inspect(&self) -> Node {
        self.node(self.tree().root())
    }

    fn node(&self, id: NodeId) -> Node {
        let tree = self.tree();
        let widget = tree.widget(id);
        let rect = tree.rect(id);
        let mut node = Node {
            id: widget.common().id.clone(),
            kind: widget.kind(),
            rect: [rect.x0, rect.y0, rect.width(), rect.height()],
            in_view: rect.is_zero_area() || {
                let visible = tree.visible_rect(id);
                visible.width() > 0.0 && visible.height() > 0.0
            },
            offset: None,
            content: None,
            state: None,
            enabled: untrack(|| widget.enabled()),
            visible: self.shown(id),
            open: tree.layer(id).map(|_| tree.shown(id)),
            focused: tree.focused() == Some(id),
            title: untrack(|| widget.title()),
            label: None,
            variant: None,
            text: None,
            trimmed: false,
            background: None,
            corner_radius: None,
            placeholder: None,
            password: widget.text_input() == Some(InputPurpose::Password),
            value: None,
            range: untrack(|| widget.range()).map(|r| [*r.start(), *r.end()]),
            checked: None,
            selected: None,
            severity: None,
            message: None,
            icon: None,
            badge: None,
            status: None,
            alt: None,
            decorative: widget.decorative(),
            error: None,
            natural_size: None,
            cols: None,
            rows: None,
            cursor: None,
            history: None,
            layout: None,
            panes: None,
            active_pane: None,
            zoomed: None,
            broadcast: None,
            identifying: false,
            highlighted: None,
            url: None,
            can_go_back: None,
            can_go_forward: None,
            density: {
                let density = tree.density(id);
                let inherited = tree
                    .parent(id)
                    .map_or(Density::Standard, |p| tree.density(p));
                (density != inherited).then(|| density.name())
            },
            tabs: None,
            closable: false,
            reorderable: false,
            accelerators: widget
                .common()
                .accelerators
                .iter()
                .flat_map(|a| a.shortcuts())
                .map(|shortcut| shortcut.to_string())
                .collect(),
            context_flyout: tree.context_flyout(id).is_some(),
            items: None,
            items_total: None,
            filter: None,
            properties: None,
            categories: None,
            alphabetical: None,
            changed: false,
            grid_columns: None,
            grid_sort: None,
            grid_rows: None,
            grid_selected: None,
            description: None,
            depth: None,
            children: Vec::new(),
        };
        let text = untrack(|| widget.text());
        #[cfg(feature = "terminal")]
        if let Some(panes) = widget
            .as_any()
            .and_then(|any| any.downcast_ref::<tomni_ui_terminal::TerminalPanes>())
        {
            let info = panes.info();
            node.layout = Some(info.layout);
            node.panes = Some(
                info.panes
                    .into_iter()
                    .map(|pane| PaneNodeInfo {
                        tabs: pane.tabs.into_iter().map(|(k, t)| [k, t]).collect(),
                        selected: pane.selected,
                    })
                    .collect(),
            );
            node.active_pane = Some(info.active);
            node.zoomed = info.zoomed;
            node.broadcast = Some(info.broadcast);
            node.identifying = info.identifying;
            node.highlighted = info.highlighted;
        }
        drop(widget);
        let mut link_parts: Vec<Node> = Vec::new();
        match tree.look(id) {
            Look::Button { label, visual } => {
                node.label = Some(label).filter(|l| !(visual.icon_only && l.is_empty()));
                node.icon = visual.icon.map(Symbol::name);
                node.variant = Some(match visual.variant {
                    ButtonVariant::Standard => "standard",
                    ButtonVariant::Accent => "accent",
                    ButtonVariant::Hyperlink => "hyperlink",
                });
                node.state = Some(state_name(visual.state));
            }
            Look::Edit(visual) => {
                if node.kind == "number_box" {
                    node.value = parse_number(&visual.text);
                }
                node.text = Some(visual.text);
                node.placeholder = Some(visual.placeholder).filter(|p| !p.is_empty());
                node.state = Some(match visual.state {
                    EditState::Rest => "rest",
                    EditState::Hover => "hover",
                    EditState::Focused => "focused",
                    EditState::Disabled => "disabled",
                });
            }
            Look::TextEditor(visual) => {
                node.text = Some(visual.text.to_string());
                node.placeholder = Some(visual.placeholder).filter(|p| !p.is_empty());
                node.state = Some(match visual.state {
                    EditState::Rest => "rest",
                    EditState::Hover => "hover",
                    EditState::Focused => "focused",
                    EditState::Disabled => "disabled",
                });
            }
            Look::Text(visual) => {
                let rect = tree.rect(id);
                node.trimmed = self.drawer().text_trimmed(rect, &visual);
                node.variant = Some(visual.ramp.name());
                node.text = Some(visual.text);
            }
            Look::RichText(visual) => {
                let rect = tree.rect(id);
                node.trimmed = self.drawer().rich_text_trimmed(rect, &visual);
                node.variant = Some(visual.ramp.name());
                let text = visual.text();
                let links = visual.links();
                let rects = self.drawer().rich_text_links(rect, &visual);
                for (n, link) in links.iter().enumerate() {
                    let Some(first) = rects.get(n).and_then(|r| r.first()) else {
                        continue;
                    };
                    let first = *first + rect.origin().to_vec2();
                    let mut part = node.clone();
                    part.id = visual.link_id(n);
                    part.kind = "link";
                    part.rect = [first.x0, first.y0, first.width(), first.height()];
                    part.text = Some(text[link.range.clone()].to_owned());
                    part.url = Some(link.url.clone());
                    part.trimmed = false;
                    part.variant = None;
                    part.focused = node.focused && visual.focused == Some(n);
                    part.state = Some(if visual.pressed == Some(n) {
                        "pressed"
                    } else if visual.hovered == Some(n) {
                        "hover"
                    } else {
                        "rest"
                    });
                    link_parts.push(part);
                }
                node.text = Some(text);
            }
            Look::Border(visual) => {
                node.background = visual.background.as_ref().map(Brush::describe);
                node.corner_radius = Some(visual.corner_radius);
            }
            Look::Card(visual) => {
                node.background = Some(
                    match visual.state {
                        ButtonState::Rest => "CardBackgroundFillColorDefault",
                        ButtonState::Hover => "ControlFillColorSecondary",
                        ButtonState::Pressed => "ControlFillColorTertiary",
                        ButtonState::Disabled => "ControlFillColorDisabled",
                    }
                    .to_owned(),
                );
                node.corner_radius = Some(CARD_CORNER_RADIUS);
                node.label = text;
                if visual.clickable {
                    node.state = Some(state_name(visual.state));
                }
            }
            Look::SettingsCard(visual) => {
                node.background = Some(
                    match visual.state {
                        ButtonState::Rest => "CardBackgroundFillColorDefault",
                        ButtonState::Hover => "ControlFillColorSecondary",
                        ButtonState::Pressed => "ControlFillColorTertiary",
                        ButtonState::Disabled => "ControlFillColorDisabled",
                    }
                    .to_owned(),
                );
                node.corner_radius = Some(SETTINGS_CARD_CORNER_RADIUS);
                node.label = text;
                if visual.clickable {
                    node.state = Some(state_name(visual.state));
                }
            }
            Look::ToggleButton { label, visual } => {
                node.label = Some(label).filter(|l| !(visual.icon_only && l.is_empty()));
                node.icon = visual.icon.map(Symbol::name);
                node.checked = Some(visual.checked);
                node.state = Some(state_name(visual.state));
            }
            Look::CheckBox { label, visual } => {
                node.label = Some(label);
                node.checked = Some(visual.checked);
                node.state = Some(state_name(visual.state));
            }
            Look::ToggleSwitch { header, visual } => {
                node.label = Some(header).filter(|h| !h.is_empty());
                node.checked = Some(visual.on);
                node.state = Some(state_name(visual.state));
            }
            Look::InfoBar(visual) => {
                node.title = Some(visual.title.clone()).filter(|t| !t.is_empty());
                node.message = Some(visual.message.clone()).filter(|m| !m.is_empty());
                node.severity = Some(visual.severity.name());
                node.state = visual.closable.then(|| state_name(visual.close_state));
                if let Some(label) = visual.action.clone() {
                    // O botão de ação aparece na árvore como um botão filho, com o id dele.
                    let rect = tree.rect(id);
                    let button = self.drawer().info_bar_action(rect, &visual);
                    let button = button.map(|b| b + rect.origin().to_vec2());
                    let mut part = node.clone();
                    part.id = visual.action_id.clone();
                    part.kind = "button";
                    part.rect = button.map_or(part.rect, |b| [b.x0, b.y0, b.width(), b.height()]);
                    part.label = Some(label);
                    part.state = Some(state_name(visual.action_state));
                    part.title = None;
                    part.message = None;
                    part.severity = None;
                    part.variant = Some("standard");
                    part.focused = node.focused && visual.action_focus_ring;
                    link_parts.push(part);
                }
            }
            Look::ScrollViewer(visual) => {
                let offset = visual.scroller.offset();
                let content = visual.scroller.content();
                node.offset = Some([offset.x, offset.y]);
                node.content = Some([content.width, content.height]);
                node.state = Some(match visual.state {
                    ScrollBarState::Rest => "rest",
                    ScrollBarState::Hover => "hover",
                    ScrollBarState::Pressed => "pressed",
                });
            }
            Look::ComboBox(visual) => {
                node.text = Some(visual.text);
                node.placeholder = Some(visual.placeholder).filter(|p| !p.is_empty());
                node.state = Some(state_name(visual.state));
            }
            Look::ComboBoxItem { label, visual } => {
                node.label = Some(label);
                node.selected = Some(visual.selected);
                node.state = Some(state_name(visual.state));
            }
            Look::MenuItem { label, visual } => {
                node.label = Some(label);
                node.icon = visual.icon.map(Symbol::name);
                match visual.kind {
                    MenuItemKind::Action => {}
                    MenuItemKind::Toggle { checked } | MenuItemKind::Radio { checked } => {
                        node.checked = Some(checked);
                    }
                    MenuItemKind::Submenu { open } => node.open = Some(open),
                }
                node.state = Some(state_name(visual.state));
            }
            Look::Sizer(visual) => {
                node.value = text.and_then(|t| t.parse().ok());
                node.state = Some(state_name(visual.state));
            }
            Look::MenuBarItem { label, visual } => {
                node.label = Some(label);
                node.open = Some(visual.open);
                node.state = Some(state_name(visual.state));
            }
            Look::RadioButton { label, visual } => {
                node.label = Some(label);
                node.checked = Some(visual.checked);
                node.state = Some(state_name(visual.state));
            }
            Look::Slider { header, visual } => {
                node.label = Some(header).filter(|h| !h.is_empty());
                node.value = text.and_then(|t| t.parse().ok());
                node.state = Some(state_name(visual.state));
            }
            Look::ProgressBar(visual) | Look::ProgressRing(visual) => node.value = visual.value,
            Look::Chart(visual) => node.value = visual.values.last().copied(),
            Look::LevelMeter(visual) => {
                node.value = Some(visual.value);
                node.state = Some(state_name(visual.state));
            }
            Look::Expander(visual) => {
                node.label = text;
                node.open = Some(visual.open);
                node.state = Some(state_name(visual.state));
            }
            Look::TabView(visual) => {
                node.text = text;
                node.tabs = Some(visual.tabs.into_iter().map(|(k, t)| [k, t]).collect());
                node.closable = visual.closable;
                node.reorderable = visual.reorderable;
            }
            Look::TabStripButton(visual) => {
                node.label = text;
                node.state = Some(state_name(visual.state));
            }
            Look::TabHeader { label, visual, .. } => {
                node.label = Some(label);
                node.selected = Some(visual.selected);
                node.state = Some(state_name(visual.state));
            }
            Look::FieldButton(visual) => {
                node.label = text;
                node.state = Some(state_name(visual.state));
                if let FieldGlyph::Symbol(symbol) = visual.glyph {
                    node.icon = Some(symbol.name());
                }
            }
            Look::Icon(visual) => {
                node.icon = Some(visual.symbol.name());
                node.decorative = visual.decorative;
            }
            Look::Image(visual) => {
                node.alt = text;
                node.variant = Some(visual.stretch.name());
                node.error = self.drawer().image_error(&visual);
                node.natural_size = visual.natural_size.map(|(w, h)| [w, h]);
            }
            Look::NavigationView(visual) => {
                node.variant = Some(visual.mode.name());
                node.open = Some(visual.open);
                node.text = text;
            }
            Look::NavigationItem {
                label,
                icon,
                visual,
            } => {
                node.label = Some(label);
                node.icon = Some(icon.name());
                node.state = Some(state_name(visual.state));
                if node.kind == "navigation_item" {
                    node.selected = Some(visual.selected);
                    node.open = visual.expanded;
                }
            }
            Look::TreeView(visual) => {
                node.items_total = Some(visual.rows.rows.len());
                node.items = Some(
                    visual
                        .rows
                        .rows
                        .iter()
                        .take(MAX_TREE_ITEMS)
                        .map(|row| TreeItemInfo {
                            key: row.key.clone(),
                            label: row.label.clone(),
                            depth: row.depth,
                            expanded: row.expanded,
                            selected: visual.selected.as_deref() == Some(row.key.as_str()),
                        })
                        .collect(),
                );
                node.filter = Some(visual.filter).filter(|f| !f.is_empty());
                node.text = text;
            }
            Look::DataGrid(visual) => {
                node.items_total = Some(visual.rows.rows.len());
                node.grid_columns = Some(visual.columns.iter().map(|c| c.header.clone()).collect());
                node.grid_sort = visual.sort.as_ref().and_then(|(key, direction)| {
                    let column = visual.columns.iter().find(|c| &c.key == key)?;
                    let way = match direction {
                        tomni_ui_core::SortDirection::Ascending => "crescente",
                        tomni_ui_core::SortDirection::Descending => "decrescente",
                    };
                    Some(format!("{} {way}", column.header))
                });
                node.grid_rows = Some(
                    visual
                        .rows
                        .rows
                        .iter()
                        .take(MAX_TREE_ITEMS)
                        .map(|row| GridRowInfo {
                            key: row.key.clone(),
                            cells: row.cells.iter().map(|(text, _)| text.clone()).collect(),
                            selected: visual.selected.contains(&row.key),
                            tone: row.tone.map(|tone| tone.name().to_owned()),
                        })
                        .collect(),
                );
                node.grid_selected = Some(visual.selected);
                node.filter = Some(visual.filter).filter(|f| !f.is_empty());
            }
            Look::DataGridHeader(_) => node.text = text,
            Look::DataGridRow(visual) => {
                node.selected = Some(visual.selected);
                node.state = Some(state_name(visual.state));
                node.description = visual.row.tone.map(|tone| tone.description().to_owned());
                node.text = text;
            }
            Look::TreeViewItem {
                label,
                icon,
                badge,
                visual,
            } => {
                node.label = Some(label);
                node.icon = Some(icon.name());
                node.badge = badge.map(|b| b.name());
                node.status = visual.status.map(|s| s.name());
                node.state = Some(state_name(visual.state));
                node.selected = Some(visual.selected);
                node.open = visual.has_children.then_some(visual.expanded);
                node.depth = Some(visual.depth);
            }
            Look::PropertyGrid(visual) => {
                node.properties = Some(
                    visual
                        .properties
                        .into_iter()
                        .map(|p| PropertyNodeInfo {
                            category: p.category,
                            name: p.name,
                            value: p.value,
                            changed: p.changed,
                            selected: p.selected,
                            read_only: p.read_only,
                            enabled: p.enabled,
                            hidden: p.hidden,
                        })
                        .collect(),
                );
                node.categories = Some(
                    visual
                        .categories
                        .into_iter()
                        .map(|c| CategoryNodeInfo {
                            name: c.name,
                            expanded: c.expanded,
                            hidden: c.hidden,
                        })
                        .collect(),
                );
                node.alphabetical = Some(visual.alphabetical);
            }
            Look::PropertyGridRow {
                name,
                value,
                visual,
            } => {
                node.label = Some(name);
                node.text = Some(value);
                node.state = Some(state_name(visual.state));
                node.selected = Some(visual.selected);
                node.changed = visual.changed;
            }
            Look::PropertyGridCategory { label, visual } => {
                node.label = Some(label);
                node.state = Some(state_name(visual.state));
                node.selected = Some(visual.selected);
                node.open = Some(visual.expanded);
            }
            Look::TitleBar { active } => {
                node.text = text;
                node.state = Some(if active { "active" } else { "inactive" });
            }
            Look::TitleBarButton {
                label,
                icon,
                visual,
            } => {
                node.label = Some(label);
                node.icon = Some(icon.name());
                node.state = Some(state_name(visual.state));
            }
            Look::CaptionButton(visual) => {
                node.label = text;
                node.state = Some(state_name(visual.state));
            }
            Look::Column if node.kind == "radio_buttons" => node.text = text,
            Look::Row | Look::Column if node.kind == "split" => {
                node.variant = Some(if matches!(tree.look(id), Look::Row) {
                    "row"
                } else {
                    "column"
                });
                node.value = text.and_then(|t| t.parse().ok());
            }
            Look::Custom(custom) => {
                node.text = text;
                #[cfg(feature = "terminal")]
                if let Some(visual) = custom.downcast::<tomni_ui_terminal::TerminalVisual>() {
                    let emu = visual.emulator();
                    node.cols = Some(emu.cols());
                    node.rows = Some(emu.rows());
                    node.cursor = emu
                        .modes()
                        .cursor_visible
                        .then(|| emu.cursor())
                        .map(|(row, col)| [col, row]);
                    node.history = Some([emu.history_len(), emu.display_offset()]);
                    node.title = Some(emu.title().to_owned()).filter(|t| !t.is_empty());
                    node.variant = Some(visual.scheme().name());
                    node.state = Some(if visual.focused() { "focused" } else { "rest" });
                }
                #[cfg(feature = "webview-widget")]
                if let Some(visual) = custom.downcast::<tomni_ui_webview::WebViewVisual>() {
                    node.url = Some(visual.url.clone());
                    node.title = Some(visual.title.clone()).filter(|t| !t.is_empty());
                    node.state = Some(visual.state.name());
                    node.error = visual.error.clone();
                    node.can_go_back = Some(visual.can_go_back);
                    node.can_go_forward = Some(visual.can_go_forward);
                    node.text = None;
                }
                #[cfg(not(any(feature = "terminal", feature = "webview-widget")))]
                let _ = custom;
            }
            Look::HeatMap(_)
            | Look::Form
            | Look::Column
            | Look::Row
            | Look::Fragment
            | Look::Popup(_)
            | Look::ContentDialog { .. }
            | Look::MenuSeparator
            | Look::DragLabel(_)
            | Look::NavigationPane { .. }
            | Look::DockOverlay { .. }
            | Look::NavigationContent { .. }
            | Look::PropertyGridSizer(_) => {}
        }
        node.children = tree
            .children(id)
            .iter()
            .map(|&child| self.node(child))
            .collect();
        node.children.extend(link_parts);
        if node.kind == "radio_buttons" {
            // O cabeçalho é o `Text` da primeira coluna do grupo, oculta quando ele está vazio.
            node.label = node
                .children
                .first()
                .and_then(|header| header.children.first())
                .and_then(|text| text.text.clone())
                .filter(|header| !header.is_empty());
        }
        node
    }
}
