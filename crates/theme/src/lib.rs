//! Trait `Drawer` e o `FluentDrawer`, que pinta cada widget a partir de
//! `(tipo, estado, rect, tokens)`, com tokens claro e escuro.
//!
//! Trocar o tema é trocar o `Drawer`.

mod card;
mod checkbox;
mod data_grid;
mod drag;
mod edit;
mod expander;
mod field_button;
mod icon;
mod icons;
mod image;
mod info_bar;
mod menu_bar;
mod navigation_view;
mod popup;
mod progress;
mod property_grid;
mod radio_button;
mod rich_text;
mod scroll_viewer;
mod slider;
mod split;
mod symbols;
mod tab_view;
#[cfg(feature = "terminal")]
mod terminal;
mod text;
mod text_editor;
mod title_bar;
mod toggle_switch;
mod tokens;
mod tree_view;
#[cfg(feature = "webview")]
mod webview;

pub use edit::FluentTextMetrics;
pub use text::ramp_style;
pub use tokens::{Theme, Tokens, metrics};

use std::cell::RefCell;
use std::rc::Rc;

use tomni_ui_core::{
    BUTTON_ICON_SIZE, BUTTON_ICON_SPACING, BorderVisual, ButtonState, ButtonVariant, ButtonVisual,
    CaptionButtonVisual, CardVisual, CheckBoxVisual, ComboBoxItemVisual, ComboBoxVisual,
    CustomLook, DataGridHeaderVisual, DataGridRowVisual, DataGridVisual, Density, DockSide,
    EditVisual, ExpanderVisual, FieldButtonVisual, ICON_BUTTON_SIZE, ICON_BUTTON_SIZE_COMPACT,
    IconVisual, ImageVisual, InfoBarVisual, Look, MenuBarItemVisual, MenuItemVisual,
    NavigationItemVisual, PopupKind, ProgressVisual, PropertyGridCategoryVisual,
    PropertyGridRowVisual, PropertyGridSizerVisual, RadioButtonVisual, RichTextVisual,
    ScrollViewerVisual, SettingsCardVisual, SizerVisual, SliderVisual, Symbol, TabHeaderVisual,
    TabStripButtonVisual, TabViewVisual, TextEditorVisual, TextMetrics, TextVisual,
    TitleBarButtonVisual, ToggleSwitchVisual, TreeViewItemVisual,
};
use tomni_ui_render::kurbo::{BezPath, Point, Rect, RoundedRect, Shape, Size};
use tomni_ui_render::peniko::{Color, Gradient};
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::{Fonts, TextStyle};

/// Mede e pinta os widgets. Os widgets do core só guardam estado; tudo o que é visual passa
/// por aqui.
pub trait Drawer {
    /// Cores do tema atual.
    fn tokens(&self) -> &Tokens;

    /// Tamanho natural de um botão com o rótulo `label` (e o ícone de `visual`, se houver).
    fn measure_button(&mut self, label: &str, visual: ButtonVisual) -> Size;

    /// Pinta um botão ocupando `rect`. O anel de foco, quando há, sai até
    /// [`metrics::FOCUS_MARGIN`] px para fora de `rect`.
    fn draw_button(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: ButtonVisual,
    );

    /// Tamanho natural de um campo de texto, que não depende do texto (o que não cabe rola),
    /// só da densidade.
    fn measure_edit(&mut self, visual: &EditVisual) -> Size;

    /// Pinta um campo de texto ocupando `rect`.
    fn draw_edit(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &EditVisual);

    /// Tamanho natural de um `TextEditor`: a largura mínima de um campo e a altura mínima, ou,
    /// com `max_height`, a do texto quebrado em `max_width`, entre a mínima e a máxima.
    fn measure_text_editor(&mut self, visual: &TextEditorVisual, max_width: Option<f64>) -> Size;

    /// Pinta um `TextEditor` ocupando `rect`: a moldura, o texto das linhas visíveis, as marcas,
    /// a seleção, o cursor e as barras de rolagem.
    fn draw_text_editor(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &TextEditorVisual);

    /// Onde fica o cursor de um `TextEditor` em `rect`, na janela. `None` sem cursor.
    fn text_editor_caret_rect(&mut self, rect: Rect, visual: &TextEditorVisual) -> Option<Rect>;

    /// Medidas de texto do campo, com a mesma fonte e o mesmo padding usados para pintar. O
    /// `Ui` instala na árvore, para o `Edit` posicionar o cursor e rolar.
    fn text_metrics(&self) -> Box<dyn TextMetrics>;

    /// Onde fica o cursor de um campo em `rect`, na janela, com a altura da linha. A
    /// plataforma põe a janela de candidatos do IME ali. `None` sem cursor.
    fn caret_rect(&mut self, rect: Rect, visual: &EditVisual) -> Option<Rect>;

    /// Tamanho natural de um texto solto. Com `max_width`, as linhas quebram nessa largura
    /// (e param no `max_lines`, se houver).
    fn measure_text(&mut self, visual: &TextVisual, max_width: Option<f64>) -> Size;

    /// Pinta um texto solto em `rect`; com `wrap`, quebrando as linhas na largura dele.
    fn draw_text(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &TextVisual);

    /// Se o texto em `rect` aparece cortado com reticências (`max_lines`).
    fn text_trimmed(&self, rect: Rect, visual: &TextVisual) -> bool {
        let _ = (rect, visual);
        false
    }

    /// Tamanho natural de um texto com estilos. Com `max_width`, as linhas quebram nessa largura.
    fn measure_rich_text(&mut self, visual: &RichTextVisual, max_width: Option<f64>) -> Size;

    /// Pinta um texto com estilos em `rect`; com `wrap`, quebrando as linhas na largura dele.
    fn draw_rich_text(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &RichTextVisual);

    /// Retângulos dos links de um texto com estilos em `rect`, relativos ao canto dele: um
    /// `Vec` por link, com um retângulo por linha visual.
    fn rich_text_links(&self, rect: Rect, visual: &RichTextVisual) -> Vec<Vec<Rect>>;

    /// Se o texto com estilos em `rect` aparece cortado com reticências (`max_lines`).
    fn rich_text_trimmed(&self, rect: Rect, visual: &RichTextVisual) -> bool {
        let _ = (rect, visual);
        false
    }

    /// Onde fica o botão de ação de uma `InfoBar` em `rect`, relativo ao canto dela. `None` sem
    /// botão de ação.
    fn info_bar_action(&self, rect: Rect, visual: &InfoBarVisual) -> Option<Rect> {
        let _ = (rect, visual);
        None
    }

    /// Tamanho natural de um ícone.
    fn measure_icon(&mut self, visual: &IconVisual) -> Size;

    /// Pinta um ícone, centralizado em `rect`.
    fn draw_icon(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &IconVisual);

    /// Tamanho natural de uma imagem; com `max_width` menor, encolhida na proporção. Uma
    /// imagem que não abre mede zero.
    fn measure_image(&mut self, visual: &ImageVisual, max_width: Option<f64>) -> Size;

    /// Pinta uma imagem em `rect`, no `Stretch` dela. Uma que não abre não pinta nada.
    fn draw_image(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &ImageVisual);

    /// Por que a imagem não abre (arquivo que não existe, bytes inválidos), se não abre.
    fn image_error(&self, visual: &ImageVisual) -> Option<String> {
        let _ = visual;
        None
    }

    /// Tamanho natural de uma caixa de seleção com o rótulo `label`.
    fn measure_checkbox(&mut self, label: &str) -> Size;

    /// Pinta uma caixa de seleção ocupando `rect`.
    fn draw_checkbox(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: CheckBoxVisual,
    );

    /// Tamanho natural de uma chave liga/desliga com o rótulo de cima `header` (vazio, sem
    /// rótulo).
    fn measure_toggle_switch(&mut self, header: &str) -> Size;

    /// Pinta uma chave liga/desliga ocupando `rect`.
    fn draw_toggle_switch(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        header: &str,
        visual: ToggleSwitchVisual,
    );

    /// Tamanho natural de uma barra de aviso. Com `max_width`, o texto que não cabe numa linha
    /// quebra e a barra fica mais alta.
    fn measure_info_bar(&mut self, visual: &InfoBarVisual, max_width: Option<f64>) -> Size;

    /// Pinta uma barra de aviso ocupando `rect`.
    fn draw_info_bar(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &InfoBarVisual);

    /// Pinta a superfície de um popup: fundo, borda e sombra.
    fn draw_popup(&mut self, canvas: &mut dyn Canvas, rect: Rect, kind: PopupKind);

    /// Pinta a superfície de um `ContentDialog`, com a faixa dos botões de `footer` de altura.
    fn draw_content_dialog(&mut self, canvas: &mut dyn Canvas, rect: Rect, footer: f64);

    /// Tamanho natural de um item de menu (a altura depende da densidade).
    fn measure_menu_item(&mut self, label: &str, visual: MenuItemVisual) -> Size;

    /// Pinta um item de menu.
    fn draw_menu_item(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: MenuItemVisual,
    );

    /// Tamanho natural do separador de um menu.
    fn measure_menu_separator(&mut self) -> Size;

    /// Tamanho natural de um item da `MenuBar`.
    fn measure_menu_bar_item(&mut self, label: &str, visual: MenuBarItemVisual) -> Size;

    /// Pinta um item da `MenuBar`.
    fn draw_menu_bar_item(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: MenuBarItemVisual,
    );

    /// Tamanho natural do divisor de um `Split`: a espessura no eixo dos painéis.
    fn measure_sizer(&mut self, visual: SizerVisual) -> Size;

    /// Pinta o divisor de um `Split`.
    fn draw_sizer(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: SizerVisual);

    /// Tamanho da etiqueta que segue o ponteiro durante um arrasto.
    fn measure_drag_label(&mut self, label: &str) -> Size;

    /// Pinta a etiqueta do arrasto.
    fn draw_drag_label(&mut self, canvas: &mut dyn Canvas, rect: Rect, label: &str);

    /// Pinta o separador de um menu.
    fn draw_menu_separator(&mut self, canvas: &mut dyn Canvas, rect: Rect);

    /// Tamanho natural de um `ComboBox` fechado.
    fn measure_combo_box(&mut self, visual: &ComboBoxVisual) -> Size;

    /// Pinta um `ComboBox` fechado.
    fn draw_combo_box(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &ComboBoxVisual);

    /// Tamanho natural de um item da lista do `ComboBox` (a altura depende da densidade).
    fn measure_combo_box_item(&mut self, label: &str, visual: ComboBoxItemVisual) -> Size;

    /// Pinta um item da lista do `ComboBox`.
    fn draw_combo_box_item(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: ComboBoxItemVisual,
    );

    /// Tamanho natural de uma opção de um `RadioButtons`.
    fn measure_radio_button(&mut self, label: &str) -> Size;

    /// Pinta uma opção de um `RadioButtons`: o círculo e o rótulo.
    fn draw_radio_button(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: RadioButtonVisual,
    );

    /// Tamanho natural de um slider com o rótulo de cima `header` (vazio, sem rótulo).
    fn measure_slider(&mut self, header: &str) -> Size;

    /// Pinta um slider: o rótulo, o trilho e o polegar.
    fn draw_slider(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        header: &str,
        visual: SliderVisual,
    );

    /// Tamanho natural de uma barra de progresso.
    fn measure_progress_bar(&mut self) -> Size;

    /// Pinta uma barra de progresso.
    fn draw_progress_bar(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: ProgressVisual);

    /// Tamanho natural de um anel de progresso.
    fn measure_progress_ring(&mut self) -> Size;

    /// Pinta um anel de progresso.
    fn draw_progress_ring(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: ProgressVisual);

    /// Pinta o fundo de um `Expander`: o cabeçalho, a seta e, aberto, o painel do conteúdo. O
    /// tamanho vem dos filhos.
    fn draw_expander(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: ExpanderVisual);

    /// Pinta o fundo de um `TabView`. O tamanho vem dos filhos.
    fn draw_tab_view(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &TabViewVisual);

    /// Tamanho natural de uma aba: `width`, quando o `TabView` dividiu a faixa, ou o do título.
    fn measure_tab_header(
        &mut self,
        label: &str,
        visual: TabHeaderVisual,
        width: Option<f64>,
    ) -> Size;

    /// Pinta uma aba.
    fn draw_tab_header(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: TabHeaderVisual,
    );

    /// Pinta um botão da faixa de abas (fechar, "+" ou seta).
    fn draw_tab_button(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: TabStripButtonVisual,
    );

    /// Tamanho natural de um item do `NavigationView`.
    fn measure_navigation_item(&mut self, label: &str, visual: NavigationItemVisual) -> Size;

    /// Pinta um item do `NavigationView` (ou o ☰) nos 44 px de cima de `rect`; os filhos de
    /// um grupo ficam embaixo.
    fn draw_navigation_item(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        icon: Symbol,
        visual: NavigationItemVisual,
    );

    /// Pinta uma linha do `TreeView`.
    fn draw_tree_view_item(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        icon: Symbol,
        badge: Option<Symbol>,
        visual: TreeViewItemVisual,
    );

    /// Pinta o fundo do `DataGrid` (o contorno do arrasto e o texto de quando não há linhas).
    fn draw_data_grid(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &DataGridVisual);

    /// Pinta o cabeçalho do `DataGrid`.
    fn draw_data_grid_header(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: &DataGridHeaderVisual,
    );

    /// Pinta uma linha do `DataGrid`.
    fn draw_data_grid_row(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: &DataGridRowVisual,
    );

    /// Pinta uma linha de propriedade da `PropertyGrid`.
    fn draw_property_grid_row(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        name: &str,
        value: &str,
        visual: PropertyGridRowVisual,
    );

    /// Pinta o cabeçalho de uma categoria da `PropertyGrid`.
    fn draw_property_grid_category(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: PropertyGridCategoryVisual,
    );

    /// Pinta o divisor do painel de descrição da `PropertyGrid`.
    fn draw_property_grid_sizer(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: PropertyGridSizerVisual,
    );

    /// Pinta o painel do `NavigationView`: só quando aberto por cima do conteúdo (`overlay`).
    fn draw_navigation_pane(&mut self, canvas: &mut dyn Canvas, rect: Rect, overlay: bool);

    /// Pinta o painel do `DockHost` aberto por cima do conteúdo, encostado no lado `side`.
    fn draw_dock_overlay(&mut self, canvas: &mut dyn Canvas, rect: Rect, side: DockSide);

    /// Pinta o fundo e a borda do conteúdo do `NavigationView`.
    fn draw_navigation_content(&mut self, canvas: &mut dyn Canvas, rect: Rect, minimal: bool);

    /// Tamanho natural de um botão dentro de um campo (revelar a senha, limpar, incrementar).
    fn measure_field_button(&mut self, visual: FieldButtonVisual) -> Size;

    /// Pinta um botão dentro de um campo.
    fn draw_field_button(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: FieldButtonVisual);

    /// Tamanho natural de um widget de fora do core ([`Look::Custom`], como o `Terminal`);
    /// zero no tema que não o conhece.
    fn measure_custom(&mut self, look: &CustomLook, max_width: Option<f64>) -> Size {
        let _ = (look, max_width);
        Size::ZERO
    }

    /// Pinta um widget de fora do core ([`Look::Custom`]); nada no tema que não o conhece.
    fn draw_custom(&mut self, canvas: &mut dyn Canvas, rect: Rect, look: &CustomLook) {
        let _ = (canvas, rect, look);
    }

    /// O [`Drawer::text_info`] de um widget de fora do core ([`Look::Custom`]).
    fn custom_text_info(&mut self, rect: Rect, look: &CustomLook) -> Vec<TextInfo> {
        let _ = (rect, look);
        Vec::new()
    }

    /// Onde fica o cursor de texto de um widget de fora do core ([`Look::Custom`]), para a
    /// janela de candidatos do IME; `None` para usar o widget inteiro.
    fn custom_caret_rect(&mut self, rect: Rect, look: &CustomLook) -> Option<Rect> {
        let _ = (rect, look);
        None
    }

    /// Textos que o widget mostra agora, com as cores e onde precisam caber. O lint usa para
    /// conferir contraste, texto cortado e caracteres sem glifo; vazio em quem não mostra
    /// texto.
    fn text_info(&mut self, rect: Rect, look: &Look) -> Vec<TextInfo> {
        let _ = (rect, look);
        Vec::new()
    }

    /// Ícones que o widget mostra agora e que passam informação (o `Icon` não decorativo, o
    /// botão só de ícone). O lint confere o contraste de 3:1 com o fundo (WCAG 1.4.11).
    fn graphic_info(&mut self, rect: Rect, look: &Look) -> Vec<GraphicInfo> {
        let _ = (rect, look);
        Vec::new()
    }

    /// Fundo opaco que o widget pinta embaixo do conteúdo, quando é uma superfície própria
    /// (popups e `ContentDialog`); `None` nos outros. O lint compõe sobre ele as cores dos
    /// textos de dentro.
    fn surface(&self, look: &Look) -> Option<Color> {
        let _ = look;
        None
    }

    /// Fundo sobre o qual os próximos [`Drawer::text_info`] compõem as cores: o de uma
    /// [`Drawer::surface`], ou `None` para o fundo da janela.
    fn set_backdrop(&mut self, backdrop: Option<Color>) {
        let _ = backdrop;
    }

    /// Escurece `rect` (a janela inteira) atrás de uma camada modal.
    fn draw_smoke(&self, canvas: &mut dyn Canvas, rect: Rect) {
        let _ = (canvas, rect);
    }

    /// Caracteres de `text` que as fontes do tema não têm e sairiam como um retângulo.
    fn missing_glyphs(&mut self, text: &str, style: TextStyle) -> Vec<char> {
        let _ = (text, style);
        Vec::new()
    }

    /// Tamanho natural de uma folha. Os contêineres são medidos pelo layout.
    fn measure(&mut self, look: &Look) -> Size {
        self.measure_wrapped(look, None)
    }

    /// Tamanho de uma folha com `max_width` de largura disponível. Só muda o resultado nas
    /// que quebram linhas ([`Look::reflows`]); `None` mede numa linha só.
    fn measure_wrapped(&mut self, look: &Look, max_width: Option<f64>) -> Size {
        match look {
            Look::Button { label, visual } => self.measure_button(label, *visual),
            Look::ToggleButton { label, visual } => self.measure_button(label, visual.as_button()),
            Look::Icon(visual) => self.measure_icon(visual),
            Look::Image(visual) => self.measure_image(visual, max_width),
            Look::Edit(visual) => self.measure_edit(visual),
            Look::TextEditor(visual) => self.measure_text_editor(visual, max_width),
            Look::Text(visual) => self.measure_text(visual, max_width.filter(|_| visual.wrap)),
            Look::RichText(visual) => {
                self.measure_rich_text(visual, max_width.filter(|_| visual.wrap))
            }
            Look::CheckBox { label, .. } => self.measure_checkbox(label),
            Look::ToggleSwitch { header, .. } => self.measure_toggle_switch(header),
            Look::InfoBar(visual) => self.measure_info_bar(visual, max_width),
            Look::MenuItem { label, visual } => self.measure_menu_item(label, *visual),
            Look::MenuSeparator => self.measure_menu_separator(),
            Look::MenuBarItem { label, visual } => self.measure_menu_bar_item(label, *visual),
            Look::Sizer(visual) => self.measure_sizer(*visual),
            Look::DragLabel(label) => self.measure_drag_label(label),
            Look::ComboBox(visual) => self.measure_combo_box(visual),
            Look::ComboBoxItem { label, visual } => self.measure_combo_box_item(label, *visual),
            Look::RadioButton { label, .. } => self.measure_radio_button(label),
            Look::Slider { header, .. } => self.measure_slider(header),
            Look::ProgressBar(_) => self.measure_progress_bar(),
            Look::ProgressRing(_) => self.measure_progress_ring(),
            Look::TabHeader {
                label,
                visual,
                width,
            } => self.measure_tab_header(label, *visual, *width),
            Look::TabStripButton(visual) => visual.size(),
            Look::FieldButton(visual) => self.measure_field_button(*visual),
            Look::NavigationItem { label, visual, .. } => {
                self.measure_navigation_item(label, *visual)
            }
            Look::Custom(custom) => self.measure_custom(custom, max_width),
            Look::PropertyGridSizer(_) => property_grid::measure_sizer(),
            Look::TreeView(_)
            | Look::TreeViewItem { .. }
            | Look::DataGrid(_)
            | Look::DataGridHeader(_)
            | Look::DataGridRow(_)
            | Look::PropertyGrid(_)
            | Look::PropertyGridRow { .. }
            | Look::PropertyGridCategory { .. }
            | Look::NavigationView(_)
            | Look::NavigationPane { .. }
            | Look::DockOverlay { .. }
            | Look::NavigationContent { .. }
            | Look::Expander(_)
            | Look::TabView(_)
            | Look::Form
            | Look::Column
            | Look::Row
            | Look::Border(_)
            | Look::Card(_)
            | Look::SettingsCard(_)
            | Look::ScrollViewer(_)
            | Look::Fragment
            | Look::Popup(_)
            | Look::ContentDialog { .. }
            | Look::TitleBar { .. }
            | Look::TitleBarButton { .. }
            | Look::CaptionButton(_) => Size::ZERO,
        }
    }

    /// Pinta as barras de rolagem de um `ScrollViewer` que ocupa `rect`. São pintadas depois
    /// do conteúdo, por cima dele.
    fn draw_scroll_viewer(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: &ScrollViewerVisual,
    ) {
        scroll_viewer::draw(canvas, rect, visual, self.tokens());
    }

    /// Pinta um `Border` que ocupa `rect`: o fundo e a borda. O conteúdo vem depois, por cima.
    fn draw_border(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &BorderVisual) {
        card::draw_border(canvas, rect, visual, self.tokens());
    }

    /// Pinta um `Card` que ocupa `rect`: o fundo, a borda e o anel de foco.
    fn draw_card(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: CardVisual) {
        card::draw_card(canvas, rect, visual, self.tokens());
    }

    /// Pinta um `SettingsCard` que ocupa `rect`: o fundo, a borda e o anel de foco.
    fn draw_settings_card(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: SettingsCardVisual,
    ) {
        card::draw_settings_card(canvas, rect, visual, self.tokens());
    }

    /// Pinta o voltar ou o ☰ da `TitleBar` ocupando `rect`: o fundo fica 2 px para dentro.
    fn draw_title_bar_button(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        icon: Symbol,
        visual: TitleBarButtonVisual,
    ) {
        title_bar::draw_button(canvas, rect, icon, visual, self.tokens());
    }

    /// Pinta um botão da janela (minimizar, maximizar ou restaurar, fechar) ocupando `rect`.
    fn draw_caption_button(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: CaptionButtonVisual,
    ) {
        title_bar::draw_caption(canvas, rect, visual, self.tokens());
    }

    /// Avisa que a janela é transparente (o Mica do sistema aparece atrás): o `Form` deixa de
    /// pintar o fundo. O [`Drawer::background`] continua com a cor sólida, que é a de reserva
    /// do Mica, para o lint.
    fn set_window_transparent(&mut self, transparent: bool) {
        let _ = transparent;
    }

    /// Se a janela é transparente ([`Drawer::set_window_transparent`]).
    fn window_transparent(&self) -> bool {
        false
    }

    /// Cor que um contêiner pinta no ponto `at` da janela, dentro de `rect`: o fundo do
    /// `Border` (num gradiente, a ponta mais próxima) e o do `Card`. `None` nos que não pintam
    /// fundo. O lint compõe essas cores, de fora para dentro, atrás dos textos.
    fn background(&self, rect: Rect, look: &Look, at: Point) -> Option<Color> {
        match look {
            Look::Form => Some(self.tokens().solid_background_base),
            Look::Border(visual) => visual
                .background
                .as_ref()
                .and_then(|brush| card::brush_color(brush, rect, at, self.tokens())),
            Look::Card(visual) => Some(card::card_background(*visual, self.tokens())),
            Look::SettingsCard(visual) => {
                Some(card::settings_card_background(*visual, self.tokens()))
            }
            Look::NavigationPane { overlay: true } | Look::DockOverlay { .. } => {
                Some(self.tokens().acrylic_in_app_fill_default)
            }
            Look::NavigationContent { .. } => Some(self.tokens().layer_fill_default),
            _ => None,
        }
    }

    /// Pinta um widget ocupando `rect`. O `Form` pinta o fundo; `Column`, `Row`, `Show` e
    /// `For`, nada; o `ScrollViewer`, só as barras.
    fn draw(&mut self, canvas: &mut dyn Canvas, rect: Rect, look: &Look) {
        match look {
            Look::Form if self.window_transparent() => {}
            Look::Form => canvas.fill(&rect, self.tokens().solid_background_base),
            Look::Column | Look::Row | Look::Fragment | Look::TitleBar { .. } => {}
            Look::Border(visual) => self.draw_border(canvas, rect, visual),
            Look::Card(visual) => self.draw_card(canvas, rect, *visual),
            Look::SettingsCard(visual) => self.draw_settings_card(canvas, rect, *visual),
            Look::ScrollViewer(visual) => self.draw_scroll_viewer(canvas, rect, visual),
            Look::Button { label, visual } => self.draw_button(canvas, rect, label, *visual),
            Look::ToggleButton { label, visual } => {
                self.draw_button(canvas, rect, label, visual.as_button());
            }
            Look::Edit(visual) => self.draw_edit(canvas, rect, visual),
            Look::TextEditor(visual) => self.draw_text_editor(canvas, rect, visual),
            Look::Text(visual) => self.draw_text(canvas, rect, visual),
            Look::RichText(visual) => self.draw_rich_text(canvas, rect, visual),
            Look::Icon(visual) => self.draw_icon(canvas, rect, visual),
            Look::Image(visual) => self.draw_image(canvas, rect, visual),
            Look::CheckBox { label, visual } => self.draw_checkbox(canvas, rect, label, *visual),
            Look::ToggleSwitch { header, visual } => {
                self.draw_toggle_switch(canvas, rect, header, *visual)
            }
            Look::InfoBar(visual) => self.draw_info_bar(canvas, rect, visual),
            Look::Popup(kind) => self.draw_popup(canvas, rect, *kind),
            Look::ContentDialog { footer } => self.draw_content_dialog(canvas, rect, *footer),
            Look::MenuItem { label, visual } => self.draw_menu_item(canvas, rect, label, *visual),
            Look::MenuSeparator => self.draw_menu_separator(canvas, rect),
            Look::MenuBarItem { label, visual } => {
                self.draw_menu_bar_item(canvas, rect, label, *visual)
            }
            Look::Sizer(visual) => self.draw_sizer(canvas, rect, *visual),
            Look::DragLabel(label) => self.draw_drag_label(canvas, rect, label),
            Look::ComboBox(visual) => self.draw_combo_box(canvas, rect, visual),
            Look::ComboBoxItem { label, visual } => {
                self.draw_combo_box_item(canvas, rect, label, *visual)
            }
            Look::RadioButton { label, visual } => {
                self.draw_radio_button(canvas, rect, label, *visual)
            }
            Look::Slider { header, visual } => self.draw_slider(canvas, rect, header, *visual),
            Look::ProgressBar(visual) => self.draw_progress_bar(canvas, rect, *visual),
            Look::ProgressRing(visual) => self.draw_progress_ring(canvas, rect, *visual),
            Look::Expander(visual) => self.draw_expander(canvas, rect, *visual),
            Look::TabView(visual) => self.draw_tab_view(canvas, rect, visual),
            Look::TabHeader { label, visual, .. } => {
                self.draw_tab_header(canvas, rect, label, *visual)
            }
            Look::TabStripButton(visual) => self.draw_tab_button(canvas, rect, *visual),
            Look::FieldButton(visual) => self.draw_field_button(canvas, rect, *visual),
            Look::NavigationView(_) | Look::TreeView(_) | Look::PropertyGrid(_) => {}
            Look::DataGrid(visual) => self.draw_data_grid(canvas, rect, visual),
            Look::DataGridHeader(visual) => self.draw_data_grid_header(canvas, rect, visual),
            Look::DataGridRow(visual) => self.draw_data_grid_row(canvas, rect, visual),
            Look::PropertyGridRow {
                name,
                value,
                visual,
            } => self.draw_property_grid_row(canvas, rect, name, value, *visual),
            Look::PropertyGridCategory { label, visual } => {
                self.draw_property_grid_category(canvas, rect, label, *visual)
            }
            Look::PropertyGridSizer(visual) => self.draw_property_grid_sizer(canvas, rect, *visual),
            Look::TreeViewItem {
                label,
                icon,
                badge,
                visual,
            } => self.draw_tree_view_item(canvas, rect, label, *icon, *badge, *visual),
            Look::NavigationPane { overlay } => self.draw_navigation_pane(canvas, rect, *overlay),
            Look::DockOverlay { side } => self.draw_dock_overlay(canvas, rect, *side),
            Look::NavigationItem {
                label,
                icon,
                visual,
            } => self.draw_navigation_item(canvas, rect, label, *icon, *visual),
            Look::NavigationContent { minimal } => {
                self.draw_navigation_content(canvas, rect, *minimal)
            }
            Look::TitleBarButton { icon, visual, .. } => {
                self.draw_title_bar_button(canvas, rect, *icon, *visual)
            }
            Look::CaptionButton(visual) => self.draw_caption_button(canvas, rect, *visual),
            Look::Custom(custom) => self.draw_custom(canvas, rect, custom),
        }
    }
}

/// Texto pintado por um widget, visto pelo lint.
#[derive(Clone, Debug, PartialEq)]
pub struct TextInfo {
    /// O texto, como aparece (mascarado no modo senha).
    pub text: String,
    /// Cor do texto, já composta sobre `background`.
    pub color: Color,
    /// Fundo atrás do texto, opaco: o preenchimento do controle sobre o fundo da janela.
    pub background: Color,
    /// Fonte usada.
    pub style: TextStyle,
    /// Onde o texto é pintado, na janela: a largura do texto e a altura da linha.
    pub bounds: Rect,
    /// Onde o texto precisa caber. `None` quando ele rola em vez de ser cortado, como o valor
    /// de um campo de texto.
    pub clip: Option<Rect>,
}

/// Ícone pintado por um widget, visto pelo lint.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphicInfo {
    /// Nome do ícone (`Search`).
    pub name: &'static str,
    /// Cor do ícone, já composta sobre `background`.
    pub color: Color,
    /// Fundo atrás do ícone, opaco.
    pub background: Color,
    /// O quadrado do ícone, na janela.
    pub bounds: Rect,
}

/// `top` pintado sobre `bottom`, no espaço sRGB, como o render compõe.
pub fn over(top: Color, bottom: Color) -> Color {
    let [r, g, b, a] = top.components;
    let [br, bg, bb, ba] = bottom.components;
    let out = a + ba * (1.0 - a);
    if out == 0.0 {
        return Color::TRANSPARENT;
    }
    let mix = |c: f32, bc: f32| (c * a + bc * ba * (1.0 - a)) / out;
    Color::new([mix(r, br), mix(g, bg), mix(b, bb), out])
}

/// Visual Fluent 2, com os valores do WinUI.
#[derive(Debug)]
pub struct FluentDrawer {
    tokens: Tokens,
    /// Compartilhadas com as [`FluentTextMetrics`] instaladas na árvore.
    fonts: Rc<RefCell<Fonts>>,
    /// Fundo dos próximos `text_info` ([`Drawer::set_backdrop`]).
    backdrop: Option<Color>,
    /// Imagens decodificadas.
    images: RefCell<image::Images>,
    /// Layouts dos textos do `TextEditor`, compartilhados com as [`FluentTextMetrics`].
    editors: Rc<RefCell<text_editor::EditorCache>>,
    /// Janela transparente, com o Mica atrás ([`Drawer::set_window_transparent`]).
    transparent: bool,
    /// Linhas do terminal já gravadas.
    #[cfg(feature = "terminal")]
    terminal: RefCell<terminal::Cache>,
}

impl FluentDrawer {
    /// Drawer com o tema `theme`.
    pub fn new(theme: Theme) -> Self {
        Self::with_tokens(Tokens::new(theme))
    }

    /// Drawer com tokens próprios (outra cor accent, por exemplo).
    pub fn with_tokens(tokens: Tokens) -> Self {
        Self {
            tokens,
            fonts: Rc::new(RefCell::new(Fonts::new())),
            backdrop: None,
            images: RefCell::default(),
            editors: Rc::default(),
            transparent: false,
            #[cfg(feature = "terminal")]
            terminal: RefCell::default(),
        }
    }

    /// Troca o tema, mantendo as fontes carregadas.
    pub fn set_theme(&mut self, theme: Theme) {
        self.tokens = Tokens::new(theme);
    }

    /// Fundo opaco atrás do widget, para os `text_info`: o de [`Drawer::set_backdrop`] ou o da
    /// janela.
    fn backdrop(&self) -> Color {
        self.backdrop.unwrap_or(self.tokens.solid_background_base)
    }

    /// Onde ficam o ícone e o rótulo de um botão em `rect`: centralizados juntos, com
    /// [`BUTTON_ICON_SPACING`] entre eles. Devolve o quadrado do ícone, se houver, e o canto
    /// do rótulo, com a largura dele.
    fn button_content(
        &self,
        rect: Rect,
        label: &str,
        visual: ButtonVisual,
    ) -> (Option<Rect>, Point, f64) {
        let width = if visual.icon_only {
            0.0
        } else {
            let text = self.fonts.borrow_mut().layout(label, TextStyle::BODY, None);
            f64::from(text.width())
        };
        let Some(_) = visual.icon else {
            return (None, label_origin(rect, width), width);
        };
        let icon = BUTTON_ICON_SIZE;
        if visual.icon_only {
            let square = icon::centered(rect, icon);
            return (Some(square), square.origin(), 0.0);
        }
        let gap = if label.is_empty() {
            0.0
        } else {
            BUTTON_ICON_SPACING
        };
        let content = icon + gap + width;
        let origin = label_origin(rect, content);
        let square = icon::centered(rect, icon);
        let square = square.with_origin((origin.x, square.y0));
        (
            Some(square),
            Point::new(origin.x + icon + gap, origin.y),
            width,
        )
    }

    /// Rótulo do botão (e do `ToggleButton`) para o lint.
    fn button_info(&mut self, rect: Rect, label: &str, visual: ButtonVisual) -> Vec<TextInfo> {
        if visual.icon_only {
            return Vec::new();
        }
        let colors = self.button_colors(visual);
        let background = over(colors.fill, self.backdrop());
        let (_, origin, width) = self.button_content(rect, label, visual);
        let line = f64::from(TextStyle::BODY.line_height);
        let (left, _, right, _) = metrics::BUTTON_PADDING;
        let border = metrics::BORDER;
        vec![TextInfo {
            text: label.to_owned(),
            color: over(colors.text, background),
            background,
            style: TextStyle::BODY,
            bounds: Rect::new(origin.x, origin.y, origin.x + width, origin.y + line),
            clip: Some(Rect::new(
                rect.x0 + border + left,
                rect.y0 + border,
                rect.x1 - border - right,
                rect.y1 - border,
            )),
        }]
    }

    /// Ícone do botão só de ícone para o lint. O de um botão com rótulo tem a cor do rótulo,
    /// que o lint já confere com a exigência maior, de texto.
    fn button_graphic(&mut self, rect: Rect, visual: ButtonVisual) -> Vec<GraphicInfo> {
        let Some(symbol) = visual.icon.filter(|_| visual.icon_only) else {
            return Vec::new();
        };
        let colors = self.button_colors(visual);
        let background = over(colors.fill, self.backdrop());
        vec![GraphicInfo {
            name: symbol.name(),
            color: over(colors.text, background),
            background,
            bounds: icon::centered(rect, BUTTON_ICON_SIZE),
        }]
    }

    fn button_colors(&self, visual: ButtonVisual) -> ButtonColors {
        let t = &self.tokens;
        let elevation = |secondary, default, inverted| Border::Elevation {
            secondary,
            default,
            inverted,
        };
        match visual.variant {
            ButtonVariant::Standard => {
                // `ControlElevationBorderBrush`: no claro a borda mais marcada fica embaixo; no
                // escuro, em cima.
                let rest_border = elevation(
                    t.control_stroke_secondary,
                    t.control_stroke_default,
                    t.theme == Theme::Light,
                );
                let (fill, text, border) = match visual.state {
                    ButtonState::Rest => (t.control_fill_default, t.text_fill_primary, rest_border),
                    ButtonState::Hover => {
                        (t.control_fill_secondary, t.text_fill_primary, rest_border)
                    }
                    ButtonState::Pressed => (
                        t.control_fill_tertiary,
                        t.text_fill_secondary,
                        Border::Solid(t.control_stroke_default),
                    ),
                    ButtonState::Disabled => (
                        t.control_fill_disabled,
                        t.text_fill_disabled,
                        Border::Solid(t.control_stroke_default),
                    ),
                };
                ButtonColors { fill, text, border }
            }
            ButtonVariant::Accent => {
                // `AccentControlElevationBorderBrush`: invertido nos dois temas.
                let rest_border = elevation(
                    t.control_stroke_on_accent_secondary,
                    t.control_stroke_on_accent_default,
                    true,
                );
                let (fill, text, border) = match visual.state {
                    ButtonState::Rest => {
                        (t.accent_fill_default, t.text_on_accent_primary, rest_border)
                    }
                    ButtonState::Hover => (
                        t.accent_fill_secondary,
                        t.text_on_accent_primary,
                        rest_border,
                    ),
                    ButtonState::Pressed => (
                        t.accent_fill_tertiary,
                        t.text_on_accent_secondary,
                        Border::None,
                    ),
                    ButtonState::Disabled => (
                        t.accent_fill_disabled,
                        t.text_on_accent_disabled,
                        Border::None,
                    ),
                };
                ButtonColors { fill, text, border }
            }
            ButtonVariant::Hyperlink => {
                // `HyperlinkButton`: texto accent, sem borda, e fundo só com o ponteiro.
                let (fill, text) = match visual.state {
                    ButtonState::Rest => (Color::TRANSPARENT, t.accent_text_primary),
                    ButtonState::Hover => (t.subtle_fill_secondary, t.accent_text_secondary),
                    ButtonState::Pressed => (t.subtle_fill_tertiary, t.accent_text_tertiary),
                    ButtonState::Disabled => (Color::TRANSPARENT, t.accent_text_disabled),
                };
                ButtonColors {
                    fill,
                    text,
                    border: Border::None,
                }
            }
        }
    }
}

impl Drawer for FluentDrawer {
    fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    #[allow(unused_variables)]
    fn measure_custom(&mut self, look: &CustomLook, max_width: Option<f64>) -> Size {
        #[cfg(feature = "terminal")]
        if let Some(visual) = look.downcast::<tomni_ui_terminal::TerminalVisual>() {
            return terminal::measure(visual);
        }
        #[cfg(feature = "webview")]
        if look.downcast::<tomni_ui_webview::WebViewVisual>().is_some() {
            return webview::measure();
        }
        Size::ZERO
    }

    #[allow(unused_variables)]
    fn draw_custom(&mut self, canvas: &mut dyn Canvas, rect: Rect, look: &CustomLook) {
        #[cfg(feature = "terminal")]
        if let Some(visual) = look.downcast::<tomni_ui_terminal::TerminalVisual>() {
            terminal::draw(
                canvas,
                rect,
                visual,
                &self.tokens,
                &mut self.fonts.borrow_mut(),
                &mut self.terminal.borrow_mut(),
            );
        }
        #[cfg(feature = "terminal")]
        if let Some(visual) = look.downcast::<tomni_ui_terminal::PaneVisual>() {
            terminal::draw_pane(canvas, rect, visual, &self.tokens);
        }
        #[cfg(feature = "webview")]
        if let Some(visual) = look.downcast::<tomni_ui_webview::WebViewVisual>() {
            webview::draw(self, canvas, rect, visual);
        }
    }

    #[allow(unused_variables)]
    fn custom_text_info(&mut self, rect: Rect, look: &CustomLook) -> Vec<TextInfo> {
        #[cfg(feature = "terminal")]
        if let Some(visual) = look.downcast::<tomni_ui_terminal::TerminalVisual>() {
            return vec![terminal::text_info(
                rect,
                visual,
                &self.tokens,
                self.backdrop(),
            )];
        }
        #[cfg(feature = "webview")]
        if let Some(visual) = look.downcast::<tomni_ui_webview::WebViewVisual>() {
            return webview::text_info(self, rect, visual);
        }
        Vec::new()
    }

    #[allow(unused_variables)]
    fn custom_caret_rect(&mut self, rect: Rect, look: &CustomLook) -> Option<Rect> {
        #[cfg(feature = "terminal")]
        if let Some(visual) = look.downcast::<tomni_ui_terminal::TerminalVisual>() {
            return terminal::caret_rect(rect, visual);
        }
        None
    }

    fn measure_button(&mut self, label: &str, visual: ButtonVisual) -> Size {
        if visual.icon_only {
            return match visual.density {
                Density::Standard => Size::new(ICON_BUTTON_SIZE, ICON_BUTTON_SIZE),
                Density::Compact => Size::from(ICON_BUTTON_SIZE_COMPACT),
            };
        }
        let (left, _, right, _) = metrics::BUTTON_PADDING;
        let text = self.fonts.borrow_mut().layout(label, TextStyle::BODY, None);
        let icon = match (visual.icon, label.is_empty()) {
            (None, _) => 0.0,
            (Some(_), true) => BUTTON_ICON_SIZE,
            (Some(_), false) => BUTTON_ICON_SIZE + BUTTON_ICON_SPACING,
        };
        Size::new(
            (f64::from(text.width()) + icon + left + right + 2.0 * metrics::BORDER).ceil(),
            metrics::CONTROL_HEIGHT,
        )
    }

    fn draw_button(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: ButtonVisual,
    ) {
        let colors = self.button_colors(visual);
        let radius = metrics::CONTROL_CORNER_RADIUS;
        let border = metrics::BORDER;

        // `BackgroundSizing`: no standard o fundo fica dentro da borda, porque as duas cores são
        // translúcidas e não podem se somar; no accent ele passa por baixo dela.
        let inner = rect.inset(-border);
        let background = match visual.variant {
            ButtonVariant::Standard => RoundedRect::from_rect(inner, radius - border),
            ButtonVariant::Accent | ButtonVariant::Hyperlink => {
                RoundedRect::from_rect(rect, radius)
            }
        };
        canvas.fill(&background, colors.fill);

        let ring = ring(rect, radius, inner, radius - border);
        match colors.border {
            Border::None => {}
            Border::Solid(color) => canvas.fill_path(&ring, &Paint::Solid(color)),
            Border::Elevation {
                secondary,
                default,
                inverted,
            } => {
                let h = metrics::ELEVATION_BORDER_HEIGHT;
                let (start, end) = if inverted {
                    (Point::new(0.0, rect.y1), Point::new(0.0, rect.y1 - h))
                } else {
                    (Point::new(0.0, rect.y0), Point::new(0.0, rect.y0 + h))
                };
                let gradient = Gradient::new_linear(start, end)
                    .with_stops([(metrics::ELEVATION_BORDER_STOP, secondary), (1.0, default)]);
                canvas.fill_path(&ring, &Paint::Gradient(gradient));
            }
        }

        let (square, origin, _) = self.button_content(rect, label, visual);
        if let (Some(symbol), Some(square)) = (visual.icon, square) {
            icon::draw_symbol(canvas, symbol, square, colors.text);
        }
        if !visual.icon_only {
            let text = self.fonts.borrow_mut().layout(label, TextStyle::BODY, None);
            text.draw(canvas, origin, &Paint::Solid(colors.text));
        }

        if visual.focus_ring {
            draw_focus_ring(canvas, rect, radius, &self.tokens);
        }
    }

    fn measure_icon(&mut self, visual: &IconVisual) -> Size {
        icon::measure(visual)
    }

    fn draw_icon(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &IconVisual) {
        icon::draw(self, canvas, rect, visual);
    }

    fn measure_image(&mut self, visual: &ImageVisual, max_width: Option<f64>) -> Size {
        image::measure(self, visual, max_width)
    }

    fn draw_image(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &ImageVisual) {
        image::draw(self, canvas, rect, visual);
    }

    fn image_error(&self, visual: &ImageVisual) -> Option<String> {
        image::error(self, visual)
    }

    fn measure_edit(&mut self, visual: &EditVisual) -> Size {
        Size::new(
            metrics::TEXT_CONTROL_MIN_WIDTH,
            control_height(visual.density),
        )
    }

    fn draw_edit(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &EditVisual) {
        edit::draw(self, canvas, rect, visual);
    }

    fn measure_text_editor(&mut self, visual: &TextEditorVisual, max_width: Option<f64>) -> Size {
        text_editor::measure(self, visual, max_width)
    }

    fn draw_text_editor(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &TextEditorVisual) {
        text_editor::draw(self, canvas, rect, visual);
    }

    fn text_editor_caret_rect(&mut self, rect: Rect, visual: &TextEditorVisual) -> Option<Rect> {
        text_editor::caret_rect(self, rect, visual)
    }

    fn text_metrics(&self) -> Box<dyn TextMetrics> {
        Box::new(FluentTextMetrics::shared(
            Rc::clone(&self.fonts),
            Rc::clone(&self.editors),
        ))
    }

    fn caret_rect(&mut self, rect: Rect, visual: &EditVisual) -> Option<Rect> {
        edit::caret_rect(self, rect, visual)
    }

    fn measure_text(&mut self, visual: &TextVisual, max_width: Option<f64>) -> Size {
        text::measure(self, visual, max_width)
    }

    fn draw_text(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &TextVisual) {
        text::draw(self, canvas, rect, visual);
    }

    fn text_trimmed(&self, rect: Rect, visual: &TextVisual) -> bool {
        text::trimmed(self, rect, visual)
    }

    fn measure_rich_text(&mut self, visual: &RichTextVisual, max_width: Option<f64>) -> Size {
        rich_text::measure(self, visual, max_width)
    }

    fn draw_rich_text(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &RichTextVisual) {
        rich_text::draw(self, canvas, rect, visual);
    }

    fn rich_text_links(&self, rect: Rect, visual: &RichTextVisual) -> Vec<Vec<Rect>> {
        let width = visual.wrap.then(|| rect.width() + 0.5);
        rich_text::links(&self.fonts, visual, width)
    }

    fn rich_text_trimmed(&self, rect: Rect, visual: &RichTextVisual) -> bool {
        rich_text::trimmed(self, rect, visual)
    }

    fn info_bar_action(&self, rect: Rect, visual: &InfoBarVisual) -> Option<Rect> {
        info_bar::action_rect(&self.fonts, visual, rect.width())
    }

    fn measure_checkbox(&mut self, label: &str) -> Size {
        checkbox::measure(self, label)
    }

    fn draw_checkbox(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: CheckBoxVisual,
    ) {
        checkbox::draw(self, canvas, rect, label, visual);
    }

    fn measure_toggle_switch(&mut self, header: &str) -> Size {
        toggle_switch::measure(self, header)
    }

    fn draw_toggle_switch(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        header: &str,
        visual: ToggleSwitchVisual,
    ) {
        toggle_switch::draw(self, canvas, rect, header, visual);
    }

    fn measure_info_bar(&mut self, visual: &InfoBarVisual, max_width: Option<f64>) -> Size {
        info_bar::measure(self, visual, max_width)
    }

    fn draw_info_bar(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &InfoBarVisual) {
        info_bar::draw(self, canvas, rect, visual);
    }

    fn draw_popup(&mut self, canvas: &mut dyn Canvas, rect: Rect, kind: PopupKind) {
        popup::draw_popup(canvas, rect, kind, &self.tokens);
    }

    fn draw_content_dialog(&mut self, canvas: &mut dyn Canvas, rect: Rect, footer: f64) {
        popup::draw_dialog(canvas, rect, footer, &self.tokens);
    }

    fn measure_menu_item(&mut self, label: &str, visual: MenuItemVisual) -> Size {
        popup::measure_menu_item(self, label, visual)
    }

    fn draw_menu_item(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: MenuItemVisual,
    ) {
        popup::draw_menu_item(self, canvas, rect, label, visual);
    }

    fn measure_menu_separator(&mut self) -> Size {
        popup::measure_separator()
    }

    fn measure_menu_bar_item(&mut self, label: &str, _: MenuBarItemVisual) -> Size {
        menu_bar::measure_item(self, label)
    }

    fn draw_menu_bar_item(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: MenuBarItemVisual,
    ) {
        menu_bar::draw_item(self, canvas, rect, label, visual);
    }

    fn measure_sizer(&mut self, visual: SizerVisual) -> Size {
        split::measure(visual)
    }

    fn draw_sizer(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: SizerVisual) {
        split::draw(self, canvas, rect, visual);
    }

    fn measure_drag_label(&mut self, label: &str) -> Size {
        drag::measure(self, label)
    }

    fn draw_drag_label(&mut self, canvas: &mut dyn Canvas, rect: Rect, label: &str) {
        drag::draw(self, canvas, rect, label);
    }

    fn draw_menu_separator(&mut self, canvas: &mut dyn Canvas, rect: Rect) {
        popup::draw_separator(canvas, rect, &self.tokens);
    }

    fn measure_combo_box(&mut self, visual: &ComboBoxVisual) -> Size {
        popup::measure_combo_box(visual)
    }

    fn draw_combo_box(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &ComboBoxVisual) {
        popup::draw_combo_box(self, canvas, rect, visual);
    }

    fn measure_combo_box_item(&mut self, label: &str, visual: ComboBoxItemVisual) -> Size {
        popup::measure_combo_item(self, label, visual)
    }

    fn draw_combo_box_item(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: ComboBoxItemVisual,
    ) {
        popup::draw_combo_item(self, canvas, rect, label, visual);
    }

    fn measure_radio_button(&mut self, label: &str) -> Size {
        radio_button::measure(self, label)
    }

    fn draw_radio_button(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: RadioButtonVisual,
    ) {
        radio_button::draw(self, canvas, rect, label, visual);
    }

    fn measure_slider(&mut self, header: &str) -> Size {
        slider::measure(self, header)
    }

    fn draw_slider(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        header: &str,
        visual: SliderVisual,
    ) {
        slider::draw(self, canvas, rect, header, visual);
    }

    fn measure_progress_bar(&mut self) -> Size {
        progress::measure_bar()
    }

    fn draw_progress_bar(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: ProgressVisual) {
        progress::draw_bar(canvas, rect, visual, &self.tokens);
    }

    fn measure_progress_ring(&mut self) -> Size {
        progress::measure_ring()
    }

    fn draw_progress_ring(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: ProgressVisual) {
        progress::draw_ring(canvas, rect, visual, &self.tokens);
    }

    fn draw_expander(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: ExpanderVisual) {
        expander::draw(canvas, rect, visual, &self.tokens);
    }

    fn draw_tab_view(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &TabViewVisual) {
        tab_view::draw_view(canvas, rect, visual, &self.tokens);
    }

    fn measure_tab_header(
        &mut self,
        label: &str,
        visual: TabHeaderVisual,
        width: Option<f64>,
    ) -> Size {
        tab_view::measure_header(self, label, visual, width)
    }

    fn draw_tab_button(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: TabStripButtonVisual,
    ) {
        tab_view::draw_button(canvas, rect, visual, &self.tokens);
    }

    fn draw_tab_header(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: TabHeaderVisual,
    ) {
        tab_view::draw_header(self, canvas, rect, label, visual);
    }

    fn measure_navigation_item(&mut self, label: &str, visual: NavigationItemVisual) -> Size {
        navigation_view::measure_item(self, label, visual)
    }

    fn draw_navigation_item(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        icon: Symbol,
        visual: NavigationItemVisual,
    ) {
        navigation_view::draw_item(self, canvas, rect, label, icon, visual);
    }

    fn draw_tree_view_item(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        icon: Symbol,
        badge: Option<Symbol>,
        visual: TreeViewItemVisual,
    ) {
        tree_view::draw_item(self, canvas, rect, label, icon, badge, visual);
    }

    fn draw_data_grid(&mut self, canvas: &mut dyn Canvas, rect: Rect, visual: &DataGridVisual) {
        data_grid::draw(self, canvas, rect, visual);
    }

    fn draw_data_grid_header(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: &DataGridHeaderVisual,
    ) {
        data_grid::draw_header(self, canvas, rect, visual);
    }

    fn draw_data_grid_row(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: &DataGridRowVisual,
    ) {
        data_grid::draw_row(self, canvas, rect, visual);
    }

    fn draw_property_grid_row(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        name: &str,
        value: &str,
        visual: PropertyGridRowVisual,
    ) {
        property_grid::draw_row(self, canvas, rect, name, value, visual);
    }

    fn draw_property_grid_category(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        label: &str,
        visual: PropertyGridCategoryVisual,
    ) {
        property_grid::draw_category(self, canvas, rect, label, visual);
    }

    fn draw_property_grid_sizer(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: PropertyGridSizerVisual,
    ) {
        property_grid::draw_sizer(self, canvas, rect, visual);
    }

    fn draw_navigation_pane(&mut self, canvas: &mut dyn Canvas, rect: Rect, overlay: bool) {
        navigation_view::draw_pane(canvas, rect, overlay, &self.tokens);
    }

    fn draw_dock_overlay(&mut self, canvas: &mut dyn Canvas, rect: Rect, side: DockSide) {
        navigation_view::draw_dock_overlay(canvas, rect, side, &self.tokens);
    }

    fn draw_navigation_content(&mut self, canvas: &mut dyn Canvas, rect: Rect, minimal: bool) {
        navigation_view::draw_content(canvas, rect, minimal, &self.tokens);
    }

    fn measure_field_button(&mut self, visual: FieldButtonVisual) -> Size {
        field_button::measure(visual)
    }

    fn draw_field_button(
        &mut self,
        canvas: &mut dyn Canvas,
        rect: Rect,
        visual: FieldButtonVisual,
    ) {
        field_button::draw(canvas, rect, visual, &self.tokens);
    }

    fn missing_glyphs(&mut self, text: &str, style: TextStyle) -> Vec<char> {
        self.fonts.borrow_mut().missing_glyphs(text, style)
    }

    fn graphic_info(&mut self, rect: Rect, look: &Look) -> Vec<GraphicInfo> {
        match look {
            Look::Icon(visual) => icon::graphic_info(self, rect, visual).into_iter().collect(),
            Look::Button { visual, .. } => self.button_graphic(rect, *visual),
            Look::ToggleButton { visual, .. } => self.button_graphic(rect, visual.as_button()),
            Look::NavigationItem { icon, visual, .. } => {
                vec![navigation_view::item_graphic(self, rect, *icon, *visual)]
            }
            Look::TreeViewItem {
                label,
                icon,
                badge,
                visual,
            } => tree_view::item_graphic(self, rect, label, *icon, *badge, *visual),
            Look::DataGridRow(visual) => data_grid::row_graphic(self, rect, visual),
            Look::TitleBarButton { icon, visual, .. } => {
                title_bar::button_graphic(self, rect, *icon, *visual)
            }
            Look::CaptionButton(visual) => title_bar::caption_graphic(self, rect, *visual),
            Look::Sizer(visual) => split::graphic(self, rect, *visual),
            // A linha do divisor é um separador (o `divider_stroke` do WinUI), não um ícone.
            Look::PropertyGridSizer(_) => Vec::new(),
            _ => Vec::new(),
        }
    }

    fn text_info(&mut self, rect: Rect, look: &Look) -> Vec<TextInfo> {
        match look {
            Look::Button { label, visual } => self.button_info(rect, label, *visual),
            Look::ToggleButton { label, visual } => {
                self.button_info(rect, label, visual.as_button())
            }
            Look::Edit(visual) => edit::text_info(self, rect, visual).into_iter().collect(),
            Look::TextEditor(visual) => text_editor::text_info(self, rect, visual),
            Look::Text(visual) => text::text_info(self, rect, visual),
            Look::RichText(visual) => rich_text::text_info(self, rect, visual),
            Look::CheckBox { label, visual } => checkbox::text_info(self, rect, label, *visual),
            Look::ToggleSwitch { header, visual } => {
                toggle_switch::text_info(self, rect, header, *visual)
            }
            Look::InfoBar(visual) => info_bar::text_info(self, rect, visual),
            Look::MenuItem { label, visual } => {
                vec![popup::menu_item_info(self, rect, label, *visual)]
            }
            Look::MenuBarItem { label, visual } => menu_bar::item_info(self, rect, label, *visual),
            Look::DragLabel(label) => drag::info(self, rect, label),
            Look::ComboBox(visual) => popup::combo_box_info(self, rect, visual)
                .into_iter()
                .collect(),
            Look::ComboBoxItem { label, visual } => {
                vec![popup::combo_item_info(self, rect, label, *visual)]
            }
            Look::RadioButton { label, visual } => {
                radio_button::text_info(self, rect, label, *visual)
            }
            Look::Slider { header, visual } => slider::text_info(self, rect, header, *visual),
            Look::TabHeader { label, visual, .. } => {
                tab_view::header_info(self, rect, label, *visual)
            }
            Look::NavigationItem { label, visual, .. } => {
                navigation_view::item_info(self, rect, label, *visual)
            }
            Look::TreeViewItem { label, visual, .. } => {
                tree_view::item_info(self, rect, label, *visual)
            }
            Look::DataGrid(visual) => data_grid::info(self, rect, visual),
            Look::DataGridHeader(visual) => data_grid::header_info(self, rect, visual),
            Look::DataGridRow(visual) => data_grid::row_info(self, rect, visual),
            Look::PropertyGridRow {
                name,
                value,
                visual,
            } => property_grid::row_info(self, rect, name, value, *visual),
            Look::PropertyGridCategory { label, visual } => {
                property_grid::category_info(self, rect, label, *visual)
            }
            Look::Custom(custom) => self.custom_text_info(rect, custom),
            Look::PropertyGrid(_)
            | Look::PropertyGridSizer(_)
            | Look::TreeView(_)
            | Look::NavigationView(_)
            | Look::NavigationPane { .. }
            | Look::DockOverlay { .. }
            | Look::NavigationContent { .. }
            | Look::ProgressBar(_)
            | Look::ProgressRing(_)
            | Look::FieldButton(_)
            | Look::Expander(_)
            | Look::TabView(_)
            | Look::TabStripButton(_)
            | Look::Form
            | Look::Column
            | Look::Row
            | Look::Border(_)
            | Look::Card(_)
            | Look::SettingsCard(_)
            | Look::ScrollViewer(_)
            | Look::Fragment
            | Look::Popup(_)
            | Look::ContentDialog { .. }
            | Look::MenuSeparator
            | Look::Sizer(_)
            | Look::TitleBar { .. }
            | Look::TitleBarButton { .. }
            | Look::CaptionButton(_)
            | Look::Icon(_)
            | Look::Image(_) => Vec::new(),
        }
    }

    fn surface(&self, look: &Look) -> Option<Color> {
        match look {
            Look::Popup(_) => Some(popup::popup_surface(&self.tokens)),
            Look::ContentDialog { .. } => Some(popup::dialog_surface(&self.tokens)),
            _ => None,
        }
    }

    fn set_backdrop(&mut self, backdrop: Option<Color>) {
        self.backdrop = backdrop;
    }

    fn set_window_transparent(&mut self, transparent: bool) {
        self.transparent = transparent;
    }

    fn window_transparent(&self) -> bool {
        self.transparent
    }

    fn draw_smoke(&self, canvas: &mut dyn Canvas, rect: Rect) {
        popup::draw_smoke(canvas, rect, &self.tokens);
    }
}

/// Quanto o conteúdo desce num controle mais alto que [`metrics::CONTROL_HEIGHT`] (ou sobe,
/// num mais baixo): o conteúdo fica centralizado na vertical, como o `VerticalContentAlignment`
/// do WinUI.
pub(crate) fn center_offset(rect: Rect) -> f64 {
    center_offset_from(rect, metrics::CONTROL_HEIGHT)
}

/// Como [`center_offset`], num controle cuja altura natural é `natural`.
pub(crate) fn center_offset_from(rect: Rect, natural: f64) -> f64 {
    ((rect.height() - natural) / 2.0).floor()
}

/// Altura natural dos campos e do `ComboBox` na densidade `density`: 32, ou 24 na compacta.
pub(crate) fn control_height(density: Density) -> f64 {
    match density {
        Density::Standard => metrics::CONTROL_HEIGHT,
        Density::Compact => metrics::CONTROL_HEIGHT_COMPACT,
    }
}

/// Canto superior esquerdo do rótulo de um botão: centralizado nos dois eixos.
fn label_origin(rect: Rect, width: f64) -> Point {
    let (_, top, _, _) = metrics::BUTTON_PADDING;
    let x = rect.x0 + ((rect.width() - width) / 2.0).round();
    Point::new(x, rect.y0 + metrics::BORDER + top + center_offset(rect))
}

/// Anel de foco do WinUI: 2 px na cor externa e, colado ao controle, 1 px na interna.
fn draw_focus_ring(canvas: &mut dyn Canvas, rect: Rect, radius: f64, tokens: &Tokens) {
    let inner = metrics::FOCUS_INNER;
    let outer = metrics::FOCUS_MARGIN;
    let middle = outer - metrics::FOCUS_OUTER;
    debug_assert_eq!(middle, inner);
    canvas.fill_path(
        &ring(
            rect.inflate(outer, outer),
            radius + outer,
            rect.inflate(middle, middle),
            radius + middle,
        ),
        &Paint::Solid(tokens.focus_stroke_outer),
    );
    canvas.fill_path(
        &ring(rect.inflate(inner, inner), radius + inner, rect, radius),
        &Paint::Solid(tokens.focus_stroke_inner),
    );
}

/// Faixa entre dois retângulos arredondados, um dentro do outro. O de dentro vai no sentido
/// contrário, então vira um buraco com a regra de preenchimento padrão (não nula).
fn ring(outer: Rect, outer_radius: f64, inner: Rect, inner_radius: f64) -> BezPath {
    let mut path = RoundedRect::from_rect(outer, outer_radius).to_path(PATH_TOLERANCE);
    let hole = RoundedRect::from_rect(inner, inner_radius).to_path(PATH_TOLERANCE);
    path.extend(hole.reverse_subpaths());
    path
}

#[derive(Clone, Copy, Debug)]
struct ButtonColors {
    fill: Color,
    text: Color,
    border: Border,
}

#[derive(Clone, Copy, Debug)]
enum Border {
    None,
    Solid(Color),
    /// Gradiente de [`metrics::ELEVATION_BORDER_HEIGHT`] px a partir da borda de cima, ou da de
    /// baixo se `inverted`: `secondary` na borda e `default` no resto.
    Elevation {
        secondary: Color,
        default: Color,
        inverted: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn botao_tem_altura_de_controle_e_padding_do_winui() {
        let mut drawer = FluentDrawer::new(Theme::Light);
        let size = drawer.measure_button("Entrar", ButtonVisual::default());
        assert_eq!(size.height, 32.0);
        let mut fonts = Fonts::new();
        let text = f64::from(fonts.layout("Entrar", TextStyle::BODY, None).width());
        assert_eq!(size.width, (text + 24.0).ceil());
    }

    #[test]
    fn tokens_do_winui() {
        let rgba = |c: Color| {
            let c = c.to_rgba8();
            [c.r, c.g, c.b, c.a]
        };
        assert_eq!(
            rgba(Tokens::LIGHT.accent_fill_default),
            [0x00, 0x5F, 0xB8, 0xFF]
        );
        assert_eq!(
            rgba(Tokens::DARK.accent_fill_tertiary),
            [0x60, 0xCD, 0xFF, 0xCC]
        );
        assert_eq!(
            rgba(Tokens::LIGHT.control_fill_default),
            [0xFF, 0xFF, 0xFF, 0xB3]
        );
    }

    #[test]
    fn rotulo_centralizado_em_botao_alto() {
        let mut drawer = FluentDrawer::new(Theme::Light);
        let look = |label: &str| Look::Button {
            label: label.into(),
            visual: ButtonVisual::default(),
        };
        let normal = drawer
            .text_info(Rect::new(0.0, 0.0, 100.0, 32.0), &look("OK"))
            .remove(0);
        assert_eq!(normal.bounds.y0, 6.0);
        let alto = drawer
            .text_info(Rect::new(0.0, 0.0, 100.0, 44.0), &look("OK"))
            .remove(0);
        assert_eq!(alto.bounds.y0, 12.0);
        assert!(alto.clip.unwrap().contains_rect(alto.bounds));
        let longo = drawer
            .text_info(Rect::new(0.0, 0.0, 40.0, 32.0), &look("Um rótulo comprido"))
            .remove(0);
        assert!(!longo.clip.unwrap().contains_rect(longo.bounds));
    }

    #[test]
    fn composicao_sobre_opaco_fica_opaca() {
        let branco = Color::from_rgba8(255, 255, 255, 255);
        let preto_metade = Color::from_rgba8(0, 0, 0, 128);
        let c = over(preto_metade, branco).to_rgba8();
        assert_eq!((c.r, c.a), (127, 255));
        assert_eq!(over(Color::TRANSPARENT, branco), branco);
    }

    #[test]
    fn anel_e_um_buraco() {
        let path = ring(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            4.0,
            Rect::new(1.0, 1.0, 9.0, 9.0),
            3.0,
        );
        assert_eq!(path.winding(Point::new(5.0, 5.0)), 0);
        assert_ne!(path.winding(Point::new(0.5, 5.0)), 0);
    }
}
