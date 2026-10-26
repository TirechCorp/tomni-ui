//! Trait `Widget`, o que todo widget tem em comum e a descrição visual (`Look`) que vai ao
//! `Drawer`.

use std::any::Any;
use std::fmt;
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::time::Duration;

use kurbo::{Point, Rect, Size, Vec2};

use crate::border::{BorderVisual, CardVisual};
use crate::button::ButtonVisual;
use crate::checkbox::CheckBoxVisual;
use crate::color::ThemeColor;
use crate::combo_box::{ComboBoxItemVisual, ComboBoxVisual};
use crate::cx::{EventCx, InputPurpose};
use crate::data_grid::{DataGridHeaderVisual, DataGridRowVisual, DataGridVisual};
use crate::density::Density;
use crate::drag::DragData;
use crate::edit::EditVisual;
use crate::event::{Cursor, Event, Modifiers};
use crate::expander::ExpanderVisual;
use crate::field_button::FieldButtonVisual;
use crate::flyout::{ContextFlyout, MenuFlyout, MenuItemVisual};
use crate::icon::IconVisual;
use crate::image::ImageVisual;
use crate::info_bar::InfoBarVisual;
use crate::menu_bar::MenuBarItemVisual;
use crate::native::{NativeFocus, NativePlace};
use crate::navigation_view::{NavigationItemVisual, NavigationViewVisual};
use crate::progress::ProgressVisual;
use crate::prop::{IntoProp, Prop};
use crate::property_grid::{
    PropertyGridCategoryVisual, PropertyGridRowVisual, PropertyGridSizerVisual, PropertyGridVisual,
};
use crate::radio_buttons::RadioButtonVisual;
use crate::rich_text::RichTextVisual;
use crate::scroll_viewer::{ScrollLink, ScrollViewerVisual, Scroller};
use crate::settings_card::SettingsCardVisual;
use crate::shortcut::{Accelerator, Shortcut};
use crate::slider::SliderVisual;
use crate::split::SizerVisual;
use crate::symbol::Symbol;
use crate::tab_view::{TabHeaderVisual, TabStripButtonVisual, TabViewVisual};
use crate::text::TextVisual;
use crate::text_editor::TextEditorVisual;
use crate::title_bar::{CaptionButtonVisual, PaneLink, TitleBarButtonVisual, WindowState};
use crate::toggle_button::ToggleButtonVisual;
use crate::toggle_switch::ToggleSwitchVisual;
use crate::tree_view::{TreeViewItemVisual, TreeViewVisual};

/// O que mudou para um widget que [`Widget::watches_within`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Within {
    /// O ponteiro entrou (`true`) ou saiu (`false`) do widget e dos widgets de dentro.
    Pointer(bool),
    /// O foco entrou (`true`) ou saiu (`false`) do widget e dos widgets de dentro.
    Focus(bool),
}

/// Um widget da árvore. Guarda estado e lógica; quem mede e pinta é o `Drawer`, a partir do
/// [`Look`].
#[diagnostic::on_unimplemented(
    message = "`{Self}` não é um widget do Tomni UI",
    label = "não é um widget",
    note = "widgets: `Form`, `Column`, `Row`, `UniformGrid`, `Border`, `Card`, `SettingsCard`, `SettingsExpander`, `ScrollViewer`, \
            `Text`, `Icon`, `Image`, `Button`, `ToggleButton`, `Edit`, `TextEditor`, `CheckBox`, `ToggleSwitch`, `ComboBox`, `RadioButtons`, `Slider`, `ProgressBar`, `ProgressRing`, \
            `Expander`, `TabView`, `NavigationView`, `TreeView`, `DataGrid`, `PropertyGrid`, `AutoSuggestBox`, `NumberBox`, `InfoBar`, \
            `ContentDialog`, `DockHost`, `Show` \
            e `For` (`Flyout` e `MenuFlyout` vão no `Button::flyout`, a `TitleBar` no \
            `Form::title_bar`); texto solto vai num \
            `Text::new(\"...\")`, não numa `String` ou `&str`",
    note = "uma função de tela devolve `impl View`: `fn login() -> impl View {{ Form::new(\"Login\") }}`"
)]
pub trait Widget: 'static {
    /// Nome do tipo (`"button"`, `"column"`...), usado quando o widget não tem `id`.
    fn kind(&self) -> &'static str;

    /// Id, `grow`, tamanho fixo e visibilidade.
    fn common(&self) -> &Common;

    /// Se o widget aparece agora, pelo `.visible()` dele (sem olhar os ancestrais). Um widget
    /// oculto fica na árvore, mas sai do layout, do foco, da pintura e do lint.
    fn visible(&self) -> bool {
        self.common().visible.as_ref().is_none_or(Prop::get)
    }

    /// O que o `Drawer` precisa para pintar o widget agora.
    ///
    /// A árvore chama dentro de um efeito: as propriedades reativas lidas aqui registram a
    /// dependência, e o widget é repintado só quando uma delas muda o resultado.
    fn look(&self) -> Look;

    /// Disposição dos filhos, nos contêineres. `None` numa folha, que o `Drawer` mede.
    fn flex(&self) -> Option<Flex> {
        None
    }

    /// Se o contêiner também tem um tamanho natural, que o `Drawer` mede pelo [`Look`] e o
    /// layout usa como mínimo (o `Edit` com botões dentro: os botões são filhos, mas o campo
    /// continua do tamanho de um campo).
    fn measured(&self) -> bool {
        false
    }

    /// Se o contêiner tem a árvore de layout dele, separada da do pai, como o conteúdo de um
    /// `ScrollViewer` (sem rolar). O tamanho dele fica guardado para cada largura pedida até o
    /// conteúdo mudar: num contêiner com muitos filhos assim (os blocos de um `MarkdownView`
    /// grande), mudar um filho refaz o layout só dele, mesmo quando o pai mede todos em várias
    /// larguras. Não muda onde nem de que tamanho o contêiner fica; numa folha, não faz nada.
    fn layout_boundary(&self) -> bool {
        false
    }

    /// Entrega os filhos à árvore, na montagem.
    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        Vec::new()
    }

    /// Trata um evento. Devolve `true` se o consumiu. `cx` traz o retângulo do widget, a
    /// posição do ponteiro, os modificadores, as medidas de texto e o clipboard.
    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        let _ = (event, cx);
        false
    }

    /// Ids das partes virtuais que o widget mostra na árvore das ferramentas, em ordem (os
    /// links do `RichText`): o `TestApp::click` aceita esses ids e clica no lugar da parte.
    fn part_ids(&self) -> Vec<String> {
        Vec::new()
    }

    /// Se recebe eventos de ponteiro. O hit test escolhe o widget interativo mais profundo.
    fn interactive(&self) -> bool {
        false
    }

    /// Parte de `rect` (o retângulo do widget) que recebe o ponteiro. Quase sempre o
    /// retângulo inteiro; na `InfoBar`, só o botão de fechar.
    fn hit_rect(&self, rect: Rect) -> Rect {
        rect
    }

    /// Se o ponteiro em `point` atinge o widget em `rect`. O padrão é estar em
    /// [`Widget::hit_rect`]; o `ScrollViewer` só responde sobre as barras.
    fn hits(&self, rect: Rect, point: Point) -> bool {
        self.hit_rect(rect).contains(point)
    }

    /// Se pode receber foco agora.
    fn focusable(&self) -> bool {
        false
    }

    /// Se o Tab para no widget. Num grupo que se percorre com as setas (`RadioButtons`, as
    /// abas do `TabView`), só um item é parada de Tab; os outros recebem o foco pelas setas.
    fn tab_stop(&self) -> bool {
        true
    }

    /// Se está habilitado agora.
    fn enabled(&self) -> bool {
        true
    }

    /// Título da janela, no widget raiz (o `Form`).
    fn title(&self) -> Option<String> {
        None
    }

    /// Texto do widget: o valor de um `Edit` (mesmo no modo senha), o conteúdo de um `Text`
    /// ou a mensagem de uma `InfoBar`.
    fn text(&self) -> Option<String> {
        None
    }

    /// Faixa de valores do `Slider`: o [`text`](Widget::text) dele é o valor atual.
    fn range(&self) -> Option<RangeInclusive<f64>> {
        None
    }

    /// Se o grupo dá às partes que monta (opções, abas, itens da lista) ids derivados do dele,
    /// como `tema-claro` num `RadioButtons` de id `tema`. Sem id no grupo, o lint aponta o
    /// grupo, e não cada parte.
    fn names_children(&self) -> bool {
        false
    }

    /// Se o widget recebe texto quando tem o foco, e de que tipo: a plataforma liga o IME e o
    /// teclado virtual.
    fn text_input(&self) -> Option<InputPurpose> {
        None
    }

    /// Se é um fragmento (`Show`, `For`): os filhos mudam com os dados, e o fragmento não
    /// pinta nem ocupa lugar no layout (os filhos entram direto no contêiner de cima).
    fn fragment(&self) -> bool {
        false
    }

    /// Lê, num fragmento, os valores reativos de que os filhos dependem. A árvore chama dentro
    /// de um efeito e, quando um deles muda, chama [`Widget::reconcile`].
    fn track_children(&self) {}

    /// Num fragmento, a nova lista de filhos, ou `None` se não mudou. Lido sem registrar
    /// dependências.
    fn reconcile(&mut self) -> Option<Vec<Slot>> {
        None
    }

    /// Rolagem, no `ScrollViewer`.
    fn scroller(&self) -> Option<&Scroller> {
        None
    }

    /// Rolagem, no `ScrollViewer`, para a árvore e o layout mudarem.
    fn scroller_mut(&mut self) -> Option<&mut Scroller> {
        None
    }

    /// Ligação de um `ScrollViewer` comandado por fora ([`ScrollLink`]): a árvore atende os
    /// pedidos de rolagem e publica o estado nela.
    fn scroll_link(&self) -> Option<&Rc<ScrollLink>> {
        None
    }

    /// Se o widget é uma camada (um popup: `ContentDialog`, `Flyout`, `MenuFlyout`, a lista
    /// do `ComboBox`, uma dica) e como ela se comporta. A camada fica acima da árvore, junto do
    /// widget de cima (a origem), e aparece enquanto [`Widget::visible`] for `true`.
    fn layer(&self) -> Option<Layer> {
        None
    }

    /// Abre ou fecha a camada. A árvore fecha com Esc e com o clique fora (nas camadas com
    /// [`Layer::light_dismiss`]) e abre e fecha as dicas.
    fn set_open(&mut self, open: bool) {
        let _ = open;
    }

    /// Avisa que a camada saiu da pilha da árvore (fechou por qualquer caminho, inclusive pelo
    /// signal `open`, que `set_open` não vê). O `MenuFlyout` atualiza o item em realce.
    fn layer_closed(&mut self) {}

    /// Se o widget recebe o foco quando a camada em volta abre, no lugar do primeiro que
    /// aceita foco (o botão primário do `ContentDialog`, o item selecionado do `ComboBox`).
    fn autofocus(&self) -> bool {
        false
    }

    /// Se o widget anima sozinho (os indeterminados do `ProgressBar` e do `ProgressRing`, os
    /// botões que repetem enquanto estão pressionados). A árvore lê uma vez, na montagem, e
    /// passa o tempo com [`Widget::tick`] enquanto ele aparece e [`Widget::ticking`] diz que sim.
    fn animating(&self) -> bool {
        false
    }

    /// Num widget que anima, se o tempo está passando agora. Parado, ele não recebe
    /// [`Widget::tick`] nem mantém o loop de frame acordado.
    fn ticking(&self) -> bool {
        true
    }

    /// Passa `dt` de tempo na animação. A fase entra no [`Widget::look`], e a árvore põe o
    /// widget na fila de repintura depois de cada passo.
    fn tick(&mut self, dt: Duration) {
        let _ = dt;
    }

    /// Cor que o widget passa aos `Text` de dentro sem cor própria, como o `Foreground` do
    /// WinUI: a de um `Card` pressionado ou desabilitado. `None` deixa a de fora.
    fn foreground(&self) -> Option<ThemeColor> {
        None
    }

    /// Se é só enfeite (`.decorative()` na `Image`): o lint não pede texto alternativo.
    fn decorative(&self) -> bool {
        false
    }

    /// Se [`Widget::foreground`] pode mudar: o `Ui` repinta os textos de dentro quando o
    /// [`Widget::look`] do widget muda.
    fn passes_foreground(&self) -> bool {
        false
    }

    /// O layout deu ao widget o tamanho `size`, diferente do anterior. A árvore chama fora de
    /// qualquer efeito; o widget pode guardar o tamanho num signal (o `NavigationView` escolhe
    /// o modo pela largura).
    fn resized(&mut self, size: Size) {
        let _ = size;
    }

    /// Se o widget precisa saber onde o layout o pôs, para medir o que tem dentro (o
    /// `TextEditor`: a rolagem e as barras dependem do texto quebrado na largura dele). A árvore
    /// lê uma vez, na montagem, e chama [`Widget::placed`] depois de cada layout.
    fn places(&self) -> bool {
        false
    }

    /// O layout pôs o widget em `rect`. Devolve `true` se o visual mudou (a árvore o repinta).
    /// Chamado fora de qualquer efeito, depois do layout.
    fn placed(&mut self, rect: Rect, cx: &mut EventCx) -> bool {
        let _ = (rect, cx);
        false
    }

    /// Se o widget quer saber quando o ponteiro ou o foco entram e saem dele e dos widgets de
    /// dentro ([`Widget::within`]). A árvore lê uma vez, na montagem.
    fn watches_within(&self) -> bool {
        false
    }

    /// O ponteiro ou o foco entrou ou saiu do widget ou de um widget de dentro dele (o painel
    /// por cima do `DockHost` recolhe quando os dois saem). Chamado fora de qualquer efeito, só
    /// quando o estado muda.
    fn within(&mut self, change: Within) {
        let _ = change;
    }

    /// Largura que muda com o estado do widget (o painel do `NavigationView`, aberto ou
    /// fechado). A árvore chama dentro do efeito, como o [`Widget::look`]: quando um signal
    /// lido aqui muda, o layout aplica a nova largura. `None` deixa a do `.width()`.
    fn dynamic_width(&self) -> Option<f64> {
        None
    }

    /// Base e mínimo no eixo do pai que mudam com o estado (os painéis do `Split`): o widget
    /// começa com a base e encolhe até o mínimo se faltar espaço. Lido dentro do efeito, como
    /// o [`Widget::dynamic_width`].
    fn dynamic_basis(&self) -> Option<(f64, f64)> {
        None
    }

    /// Fator de crescimento no eixo do pai que muda com o estado (os painéis do `Split` por
    /// fração). Lido dentro do efeito, como o [`Widget::dynamic_basis`]. `None` deixa o
    /// `.grow()`.
    fn dynamic_grow(&self) -> Option<f64> {
        None
    }

    /// Forma do ponteiro do mouse sobre o widget, e enquanto ele segura o ponteiro. `None`
    /// deixa a seta.
    fn cursor(&self) -> Option<Cursor> {
        None
    }

    /// O que o widget arrasta: com `Some`, o botão pressionado sobre ele e movido
    /// [`DRAG_THRESHOLD`](crate::DRAG_THRESHOLD) px (ou, no toque, o dedo parado por
    /// [`HOLD_DELAY`](crate::HOLD_DELAY) e depois movido) começa um arrasto. Ele recebe
    /// [`Event::PointerCancel`] e não clica, uma etiqueta com [`DragData::label`] segue o
    /// ponteiro, e os alvos recebem [`Event::DragOver`] e [`Event::Drop`]. Só é lido quando o
    /// widget captura o ponteiro (consome o [`Event::PointerDown`]).
    fn drag_data(&self) -> Option<DragData> {
        None
    }

    /// O id do widget que deve receber o foco, se o widget pediu um (o `TerminalPanes`, depois
    /// de um atalho que divide, fecha ou troca de painel). Quem pergunta leva o pedido: a
    /// próxima chamada devolve `None`. A árvore pergunta ao dono do atalho depois de rodá-lo e a
    /// cada fragmento que refaz os filhos, e tenta como o [`EventCx::focus_later`].
    fn take_focus_request(&mut self) -> Option<String> {
        None
    }

    /// Se o widget pede o foco ao ser criado (o campo de renomear do `TreeView`). A árvore
    /// tenta como o [`EventCx::focus_later`], depois do layout.
    fn focus_on_mount(&self) -> bool {
        false
    }

    /// Se o widget fica por cima dos irmãos: é pintado depois deles e recebe o ponteiro antes,
    /// sem mudar a ordem do Tab (o painel do `NavigationView` que abre por cima do conteúdo).
    fn raised(&self) -> bool {
        false
    }

    /// Se ←, →, ↑ e ↓ que movem o foco ([`EventCx::move_focus`]) andam entre todos os widgets
    /// de dentro dele, e não só entre os irmãos (os itens do `NavigationView`, em listas
    /// diferentes).
    fn focus_scope(&self) -> bool {
        false
    }

    /// Opacidade do widget e dos descendentes, de 0 a 1, como o `Opacity` do WinUI (o ícone e
    /// o conteúdo da `TitleBar` com a janela inativa). A árvore lê dentro do efeito, como o
    /// [`Widget::look`].
    fn opacity(&self) -> f64 {
        1.0
    }

    /// Estado da janela, na `TitleBar`: a árvore guarda para o `Ui` atualizar e liga o
    /// `NavigationView` a ela.
    fn window_state(&self) -> Option<WindowState> {
        None
    }

    /// Painel que o ☰ de uma `TitleBar` pode abrir e fechar, no `NavigationView`.
    fn pane_link(&self) -> Option<PaneLink> {
        None
    }

    /// Se o widget com foco fica com o Tab e o Shift+Tab em vez de a árvore mover o foco (o
    /// `Terminal`). O Ctrl+Tab continua trocando de aba; quem captura o Tab precisa oferecer
    /// outra saída pelo teclado (o F6 do `Terminal`, com [`FocusMove::Forward`]).
    ///
    /// [`FocusMove::Forward`]: crate::FocusMove::Forward
    fn captures_tab(&self) -> bool {
        false
    }

    /// Se, com o foco neste widget, os atalhos do app (`.accelerator()`) vêm antes dele (o
    /// `Terminal`, que usaria quase toda tecla).
    fn accelerators_first(&self) -> bool {
        false
    }

    /// Se o botão direito do mouse abre o `.context_flyout()` deste widget (o padrão). Com
    /// `false`, o clique direito chega ao widget como [`Event::SecondaryDown`] (o `Terminal` com
    /// `RightClick::CopyPaste`); Shift+F10, a tecla Menu e o toque longo continuam abrindo o menu.
    fn context_on_secondary(&self) -> bool {
        true
    }

    /// Como [`Widget::context_on_secondary`], sabendo onde o botão direito foi pressionado e
    /// com quais modificadores: `rect` é o retângulo do widget e `point` o ponteiro, nas
    /// coordenadas da janela. O `Terminal` abre o menu sobre um link e cola fora dele; quando o
    /// programa pede o mouse, o botão vai ao programa e só o Shift abre o menu. O padrão chama
    /// `context_on_secondary`.
    fn context_on_secondary_at(&self, rect: Rect, point: Point, modifiers: Modifiers) -> bool {
        let _ = (rect, point, modifiers);
        self.context_on_secondary()
    }

    /// Se o menu de contexto pode abrir agora, por um caminho que não é o botão direito: Shift+F10
    /// e a tecla Menu (`at` é `None`) e o toque longo (`at` é o ponto do dedo). O `Terminal`, que
    /// sempre monta o menu para o link, só abre onde há um link ou menu do app. O padrão é
    /// `true`.
    fn context_available(&self, rect: Rect, at: Option<Point>) -> bool {
        let _ = (rect, at);
        true
    }

    /// Onde a dica (`.tooltip()`) do widget fica, na janela: em cima desse retângulo (embaixo,
    /// se não couber), em vez de junto do widget inteiro. O padrão é `None`, a dica junto do widget. Um
    /// widget grande, como o `Terminal`, usa para a dica seguir o link sob o ponteiro. A árvore
    /// pergunta de novo a cada movimento do ponteiro enquanto a dica está aberta.
    fn tip_anchor(&self, rect: Rect) -> Option<Rect> {
        let _ = rect;
        None
    }

    /// Chamado logo antes de o menu de contexto abrir e de o widget receber
    /// [`Event::ContextOpening`]: `at` é o ponto do ponteiro (botão direito ou toque longo) ou
    /// `None` quando o menu abre pelo teclado (Shift+F10, tecla Menu). Serve a quem monta o menu
    /// conforme o que está sob o ponto, como o link do `Terminal`.
    fn context_opening_at(&mut self, rect: Rect, at: Option<Point>) {
        let _ = (rect, at);
    }

    /// Se os atalhos de dentro valem com a camada fechada (o `MenuFlyout`: os itens com
    /// `.shortcut()` funcionam sem abrir o menu).
    fn accelerators_while_closed(&self) -> bool {
        false
    }

    /// Se é uma barra de menus: o F10 e o Alt solto sozinho põem o foco nela, e o ponteiro
    /// sobre ela chega aos itens mesmo com um menu dela aberto ([`Layer::pass_through`]).
    fn menu_bar(&self) -> bool {
        false
    }

    /// Se o widget recebe a roda em pixels ([`Event::Scroll`]) com o ponteiro sobre ele,
    /// mesmo sem foco, antes de o `ScrollViewer` em volta rolar.
    fn wants_scroll(&self) -> bool {
        false
    }

    /// Se o widget põe um controle do sistema por cima da área dele (o `WebView`): o `Ui`
    /// diz a cada frame onde ele aparece ([`Widget::place_native`]) e atende os pedidos de foco
    /// dele ([`Widget::take_native_focus`]). A árvore lê uma vez, na montagem.
    fn native(&self) -> bool {
        false
    }

    /// Onde o controle nativo aparece agora e se alguma coisa o cobre. O `Ui` chama fora de
    /// qualquer efeito, depois do layout, quando o lugar muda.
    fn place_native(&mut self, place: NativePlace) {
        let _ = place;
    }

    /// O pedido de foco do controle nativo desde a última chamada: ele ganhou o foco com um
    /// clique, ou o F6 pediu para sair. O `Ui` consulta a cada frame, sem pôr o widget na fila
    /// de repintura (por isso `&self`: o pedido fica num `Cell`).
    fn take_native_focus(&self) -> Option<NativeFocus> {
        None
    }

    /// A densidade que o widget impõe a si e aos descendentes (`.density()` dos contêineres).
    /// `None` herda a do pai. A árvore lê uma vez, na montagem.
    fn density(&self) -> Option<Density> {
        self.common().density
    }

    /// A árvore informa a densidade efetiva do widget, na montagem, antes de
    /// [`Widget::take_children`] e do primeiro [`Widget::look`]: as partes que um grupo monta
    /// (as opções, as abas, os itens) ajustam a altura por ela.
    fn set_density(&mut self, density: Density) {
        let _ = density;
    }

    /// O widget como `Any`, para quem conhece o tipo concreto (o `TestApp` lê a grade do
    /// `Terminal`). `None` nos widgets do core.
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }

    /// Como [`Widget::as_any`], para mudar o widget (o `feed` do `TestApp`).
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
}

/// Onde uma camada aparece em relação à origem (o widget de cima na árvore).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// Embaixo da origem, alinhada à esquerda dela; em cima, se não couber embaixo.
    #[default]
    Below,
    /// Em cima da origem, centralizada nela; embaixo, se não couber em cima (as dicas).
    Above,
    /// No centro da janela (o `ContentDialog`).
    Center,
    /// Com o canto superior esquerdo no canto superior direito da origem, deslocada por
    /// [`Layer::offset`] (o popup dos botões de incremento do `NumberBox`).
    End,
    /// À direita da origem, a [`Layer::gap`] dela, com o topo no da origem deslocado por
    /// `offset.y`; à esquerda, se não couber (o submenu de um `MenuFlyout`).
    Side,
    /// Centrada no retângulo da origem, por cima dela (o crachá com o número de um painel do
    /// `TerminalPanes`). Se não couber dentro da origem, com [`Layer::gap`] de folga de cada
    /// lado, a camada não aparece. Combinada com
    /// `input: false` e `pointer: false`, é uma **sobreposição**: pintada acima do conteúdo e
    /// abaixo dos menus e diálogos, sem receber ponteiro, teclado nem foco, e sem fechar com o
    /// clique fora.
    Over,
}

/// Comportamento de uma camada ([`Widget::layer`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    /// Escurece a janela atrás e não fecha com o clique fora.
    pub modal: bool,
    /// Fecha com o clique fora.
    pub light_dismiss: bool,
    /// Recebe o teclado e o ponteiro: enquanto está aberta, o resto da janela não recebe
    /// nada, o foco fica preso nela (o Tab dá a volta dentro dela) e, quando fecha, volta
    /// para onde estava. As dicas não recebem.
    pub input: bool,
    /// ↑, ↓, Home e End que o widget focado não usa movem o foco entre os widgets da camada,
    /// sem dar a volta (menus e a lista do `ComboBox`).
    pub arrows: bool,
    /// Sem [`Layer::input`], ainda recebe o ponteiro (a lista do `AutoSuggestBox`): o teclado
    /// e o foco ficam com a origem, o resto da janela continua recebendo tudo, e o clique fora
    /// não a fecha (quem fecha é a origem).
    pub pointer: bool,
    /// Onde aparece.
    pub placement: Placement,
    /// Pelo menos da largura da origem (a lista do `ComboBox`).
    pub match_width: bool,
    /// Distância da origem.
    pub gap: f64,
    /// Deslocamento do [`Placement::End`], como o `HorizontalOffset` e o `VerticalOffset`
    /// de um `Popup` do WinUI.
    pub offset: Vec2,
    /// Tamanho mínimo.
    pub min_size: Size,
    /// Tamanho máximo, além do da janela.
    pub max_size: Size,
    /// Fora dela, o ponteiro chega ao que está embaixo: à camada de baixo (o menu de onde
    /// saiu um submenu) ou, na camada de baixo de todas, à barra do dono
    /// ([`Widget::menu_bar`]). O hover troca de item e o clique ali não fecha nada antes de
    /// chegar.
    pub pass_through: bool,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            modal: false,
            light_dismiss: true,
            input: true,
            arrows: false,
            pointer: false,
            placement: Placement::Below,
            match_width: false,
            gap: 4.0,
            offset: Vec2::ZERO,
            min_size: Size::ZERO,
            max_size: Size::new(f64::INFINITY, f64::INFINITY),
            pass_through: false,
        }
    }
}

/// Um filho na nova lista de um fragmento ([`Widget::reconcile`]).
pub enum Slot {
    /// Mantém o filho que estava na posição `n` da lista anterior, com o estado dele.
    Keep(usize),
    /// Cria um filho novo. A closure roda num escopo reativo só dele, descartado quando o
    /// filho sai.
    New(Box<dyn FnOnce() -> Box<dyn Widget>>),
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keep(n) => f.debug_tuple("Keep").field(n).finish(),
            Self::New(_) => f.write_str("New(..)"),
        }
    }
}

/// Qualquer widget pode ser a view de um app ou filho de um contêiner.
#[diagnostic::on_unimplemented(
    message = "`{Self}` não é uma view do Tomni UI",
    label = "não é um widget",
    note = "views: `Form`, `Column`, `Row`, `UniformGrid`, `Border`, `Card`, `SettingsCard`, `SettingsExpander`, `ScrollViewer`, \
            `Text`, `Icon`, `Image`, `Button`, `ToggleButton`, `Edit`, `TextEditor`, `CheckBox`, `ToggleSwitch`, `ComboBox`, `RadioButtons`, `Slider`, `ProgressBar`, `ProgressRing`, \
            `Expander`, `TabView`, `NavigationView`, `TreeView`, `DataGrid`, `PropertyGrid`, `AutoSuggestBox`, `NumberBox`, `InfoBar`, \
            `ContentDialog`, `DockHost`, `Show` \
            e `For` (`Flyout` e `MenuFlyout` vão no `Button::flyout`, a `TitleBar` no \
            `Form::title_bar`); texto solto vai num \
            `Text::new(\"...\")`, não numa `String` ou `&str`",
    note = "uma função de tela devolve `impl View`: `fn login() -> impl View {{ Form::new(\"Login\") }}`"
)]
pub trait View: Widget {}

impl<W: Widget> View for W {}

/// Propriedades que todo widget aceita: `.id()`, `.grow()`, `.width()`, `.height()`,
/// `.visible()`, `.tooltip()`, `.accelerator()` e `.context_flyout()`.
#[derive(Clone, Debug, Default)]
pub struct Common {
    /// Id estável, usado em testes, no MCP e no lint.
    pub id: Option<String>,
    /// Parte do eixo principal do pai que o widget ocupa, como o `flex: n` do CSS.
    pub grow: f64,
    /// Largura fixa, em pixels.
    pub width: Option<f64>,
    /// Altura fixa, em pixels.
    pub height: Option<f64>,
    /// Largura máxima, em pixels (`.max_width()`).
    pub max_width: Option<f64>,
    /// Se aparece (fixo, closure ou signal). `None` é sempre visível.
    pub visible: Option<Prop<bool>>,
    /// Dica mostrada com o ponteiro parado sobre o widget ou com o foco vindo do teclado.
    pub tooltip: Option<Prop<String>>,
    /// Densidade imposta à subárvore (`.density()` dos contêineres). `None` herda a do pai.
    pub density: Option<Density>,
    /// Atalhos de teclado (`.accelerator()`), que valem enquanto o widget aparece e está
    /// habilitado.
    pub accelerators: Vec<Accelerator>,
    /// Menu de contexto (`.context_flyout()`), montado como filho do widget.
    pub context_flyout: Option<ContextFlyout>,
    /// Montado por um grupo ([`Widget::names_children`]), com o id derivado do dele.
    pub(crate) generated: bool,
}

impl Common {
    /// Propriedades de uma parte que o grupo monta (a opção de um `RadioButtons`, a aba de um
    /// `TabView`, o painel de um `TerminalPanes`), com o id derivado do id do grupo.
    pub fn generated(id: Option<String>) -> Self {
        Self {
            id,
            generated: true,
            ..Self::default()
        }
    }

    /// Se o widget foi montado por um grupo, e não escrito pelo app: o id dele vem do id do
    /// grupo ([`Widget::names_children`]).
    pub fn is_generated(&self) -> bool {
        self.generated
    }

    /// Acrescenta o atalho `shortcut`, que roda `action` (o `.accelerator()` dos widgets de
    /// fora do core, como o `Terminal`).
    pub fn add_accelerator<M>(
        &mut self,
        shortcut: impl IntoProp<Shortcut, M>,
        action: impl FnMut() + 'static,
    ) {
        self.accelerators.push(Accelerator::new(shortcut, action));
    }

    /// Usa `menu` como menu de contexto (o `.context_flyout()` dos widgets de fora do core).
    pub fn set_context_flyout(&mut self, menu: MenuFlyout) {
        self.context_flyout = Some(ContextFlyout::new(menu));
    }
}

/// Gera o builder `.density()` num contêiner com o campo `common`.
macro_rules! density_builder {
    ($widget:ty) => {
        impl $widget {
            /// Densidade dos controles de dentro: [`Density::Compact`] segue o
            /// `Compact.xaml` do WinUI (campos e `ComboBox` de 24 px, itens de 32, abas de 24)
            /// e baixa o alvo de toque que o lint confere para 24×24. Vale até outro
            /// contêiner trocar de novo.
            ///
            /// [`Density::Compact`]: $crate::Density::Compact
            pub fn density(mut self, density: $crate::Density) -> Self {
                self.common.density = Some(density);
                self
            }
        }
    };
}

pub(crate) use density_builder;

/// Gera os builders de [`Common`] num widget com o campo `common`.
macro_rules! common_builders {
    ($widget:ty) => {
        impl $widget {
            /// Id estável, usado em testes, no MCP e no lint.
            pub fn id(mut self, id: impl Into<String>) -> Self {
                self.common.id = Some(id.into());
                self
            }

            /// Ocupa o espaço do eixo principal do pai, como o `flex: n` do CSS: irmãos com
            /// `grow` dividem o espaço na proporção do valor (dois com `grow(1)` ficam do
            /// mesmo tamanho, qualquer que seja o texto). Ninguém fica menor que o próprio
            /// conteúdo. Com `.width()` numa `Row` (ou `.height()` numa `Column`), o widget
            /// parte do tamanho fixo e cresce a partir dele.
            pub fn grow(mut self, grow: impl Into<f64>) -> Self {
                self.common.grow = grow.into();
                self
            }

            /// Largura fixa, em pixels.
            pub fn width(mut self, width: impl Into<f64>) -> Self {
                self.common.width = Some(width.into());
                self
            }

            /// Altura fixa, em pixels.
            pub fn height(mut self, height: impl Into<f64>) -> Self {
                self.common.height = Some(height.into());
                self
            }

            /// Largura máxima, em pixels: o widget acompanha o espaço que o pai dá, até esse
            /// valor. Para centrar uma coluna de leitura, ponha-a numa `Row` entre dois
            /// espaçadores com `grow(1)` e dê a ela um `grow` bem maior.
            pub fn max_width(mut self, width: impl Into<f64>) -> Self {
                self.common.max_width = Some(width.into());
                self
            }

            /// Mostra ou esconde o widget (valor fixo, closure ou signal). Oculto, ele sai do
            /// layout (os irmãos ocupam o lugar), do Tab, da pintura e do lint, mas continua
            /// na árvore, com o mesmo estado.
            pub fn visible<M>(mut self, visible: impl $crate::prop::IntoProp<bool, M>) -> Self {
                self.common.visible = Some(visible.into_prop());
                self
            }

            /// Atalho de teclado que roda `action`, como o `KeyboardAccelerator` do WinUI:
            /// `.accelerator(Shortcut::ctrl('N'), move || novo())`. Vale na janela toda
            /// enquanto o widget aparece e está habilitado (num item de menu, com o menu
            /// fechado). O widget com foco trata a tecla antes (o `Edit` fica com o Ctrl+C),
            /// menos o `Terminal`, que deixa os atalhos do app passarem na frente. Com um
            /// `ContentDialog` aberto, só valem os de dentro dele. Dois widgets com o mesmo
            /// atalho são um erro do lint (`duplicate-accelerator`). O atalho também pode ser
            /// uma closure ou um signal: ele é lido na hora da tecla, então trocar o valor troca
            /// o atalho sem montar a tela de novo.
            pub fn accelerator<M>(
                mut self,
                shortcut: impl $crate::prop::IntoProp<$crate::Shortcut, M>,
                action: impl FnMut() + 'static,
            ) -> Self {
                self.common.add_accelerator(shortcut, action);
                self
            }

            /// Menu de contexto: abre com o botão direito, na posição do ponteiro, e com o
            /// Shift+F10, a tecla Menu ou o toque longo, junto do widget. Os itens com
            /// `.shortcut()` valem como atalhos mesmo com o menu fechado.
            pub fn context_flyout(mut self, menu: $crate::MenuFlyout) -> Self {
                self.common.set_context_flyout(menu);
                self
            }

            /// Dica (texto fixo, closure ou signal): aparece meio segundo depois de o ponteiro
            /// parar sobre o widget ou de ele receber o foco pelo Tab, e some quando o ponteiro
            /// sai, com um clique ou com uma tecla. Fica em cima do widget, centralizada.
            pub fn tooltip<M>(mut self, text: impl $crate::prop::IntoProp<String, M>) -> Self {
                self.common.tooltip = Some(text.into_prop());
                self
            }
        }
    };
}

pub(crate) use common_builders;

/// Descrição visual de um widget: o `Drawer` pinta a partir dela, e a árvore repinta um
/// widget só quando ela muda.
#[derive(Clone, Debug, PartialEq)]
pub enum Look {
    /// Fundo da janela.
    Form,
    /// Contêiner vertical. Não pinta nada.
    Column,
    /// Contêiner horizontal. Não pinta nada.
    Row,
    /// Botão.
    Button {
        /// Rótulo.
        label: String,
        /// Variante e estado.
        visual: ButtonVisual,
    },
    /// Botão liga/desliga.
    ToggleButton {
        /// Rótulo.
        label: String,
        /// Se está ligado e o estado.
        visual: ToggleButtonVisual,
    },
    /// Campo de texto.
    Edit(EditVisual),
    /// `TextEditor`.
    TextEditor(TextEditorVisual),
    /// Texto solto.
    Text(TextVisual),
    /// Texto com estilos por trecho e links.
    RichText(RichTextVisual),
    /// Caixa de seleção.
    CheckBox {
        /// Rótulo, à direita da caixa.
        label: String,
        /// Marcação e estado.
        visual: CheckBoxVisual,
    },
    /// Chave liga/desliga.
    ToggleSwitch {
        /// Rótulo acima da chave; vazio, sem rótulo.
        header: String,
        /// Posição e estado.
        visual: ToggleSwitchVisual,
    },
    /// Barra de aviso.
    InfoBar(InfoBarVisual),
    /// Área rolável: o `Drawer` pinta as barras, por cima do conteúdo.
    ScrollViewer(ScrollViewerVisual),
    /// `Show` ou `For`. Não pinta nada nem ocupa lugar.
    Fragment,
    /// Superfície de um popup (`Flyout`, `MenuFlyout`, lista do `ComboBox`, dica): fundo,
    /// borda e sombra.
    Popup(PopupKind),
    /// Superfície do `ContentDialog`: a parte de cima e, embaixo, a faixa dos botões, com
    /// `footer` de altura.
    ContentDialog {
        /// Altura da faixa dos botões.
        footer: f64,
    },
    /// Item de um `MenuFlyout`.
    MenuItem {
        /// Rótulo.
        label: String,
        /// Estado.
        visual: MenuItemVisual,
    },
    /// Linha que separa grupos de itens num `MenuFlyout`.
    MenuSeparator,
    /// Divisor de um `Split`.
    Sizer(SizerVisual),
    /// Etiqueta que segue o ponteiro durante um arrasto, com o texto do item. Não é de um
    /// widget: quem pinta é a `Ui`, por cima de tudo.
    DragLabel(String),
    /// Item da `MenuBar`.
    MenuBarItem {
        /// Rótulo.
        label: String,
        /// Estado.
        visual: MenuBarItemVisual,
    },
    /// Caixa do `ComboBox`, fechada.
    ComboBox(ComboBoxVisual),
    /// Item da lista do `ComboBox` (e das sugestões do `AutoSuggestBox`).
    ComboBoxItem {
        /// Rótulo.
        label: String,
        /// Estado.
        visual: ComboBoxItemVisual,
    },
    /// Uma opção de um `RadioButtons`.
    RadioButton {
        /// Rótulo, à direita do círculo.
        label: String,
        /// Marcação e estado.
        visual: RadioButtonVisual,
    },
    /// Slider.
    Slider {
        /// Rótulo acima da trilha; vazio, sem rótulo.
        header: String,
        /// Posição e estado.
        visual: SliderVisual,
    },
    /// Barra de progresso.
    ProgressBar(ProgressVisual),
    /// Anel de progresso.
    ProgressRing(ProgressVisual),
    /// Expander: o cabeçalho, com a seta, e o painel do conteúdo quando aberto. O texto do
    /// cabeçalho e o conteúdo são filhos.
    Expander(ExpanderVisual),
    /// Fundo do `TabView`: a faixa das abas e, embaixo dela, o fundo do conteúdo.
    TabView(TabViewVisual),
    /// Botão dentro de um campo: revelar a senha, limpar, aumentar e diminuir o `NumberBox`.
    FieldButton(FieldButtonVisual),
    /// Uma aba do `TabView`.
    TabHeader {
        /// Título.
        label: String,
        /// Estado.
        visual: TabHeaderVisual,
        /// Largura de todas as abas com [`TabWidth::Equal`](crate::TabWidth::Equal); `None`
        /// mede pelo título.
        width: Option<f64>,
    },
    /// Botão da faixa de abas: fechar, "+" e as setas de rolagem.
    TabStripButton(TabStripButtonVisual),
    /// Fundo e borda de um `Border`. O conteúdo é filho.
    Border(BorderVisual),
    /// Fundo e borda de um `Card`, no estado dele. O conteúdo é filho.
    Card(CardVisual),
    /// Fundo e borda de um `SettingsCard`, no estado e no lugar dele. O conteúdo é filho.
    SettingsCard(SettingsCardVisual),
    /// Ícone.
    Icon(IconVisual),
    /// Imagem.
    Image(ImageVisual),
    /// `NavigationView`: não pinta nada; o painel e o conteúdo são filhos.
    NavigationView(NavigationViewVisual),
    /// Painel do `NavigationView`: com `overlay`, aberto por cima do conteúdo (acrílico,
    /// borda e sombra).
    NavigationPane {
        /// Se está aberto por cima do conteúdo.
        overlay: bool,
    },
    /// Painel do `DockHost` aberto por cima do conteúdo, encostado no lado `side`: acrílico,
    /// borda, sombra e cantos arredondados do lado de dentro.
    DockOverlay {
        /// A borda da janela em que o painel encosta.
        side: crate::dock::DockSide,
    },
    /// Um item do `NavigationView`, ou o ☰ e a lupa do painel compacto.
    NavigationItem {
        /// Rótulo.
        label: String,
        /// Ícone.
        icon: Symbol,
        /// Estado.
        visual: NavigationItemVisual,
    },
    /// Fundo e borda do conteúdo do `NavigationView`: com `minimal`, só a linha de cima.
    NavigationContent {
        /// Se o `NavigationView` está no modo mínimo.
        minimal: bool,
    },
    /// `TitleBar` e as partes dela: não pinta nada (o fundo é o da janela). `active` muda a cor
    /// do título.
    TitleBar {
        /// Se a janela está ativa.
        active: bool,
    },
    /// O voltar e o ☰ da `TitleBar`.
    TitleBarButton {
        /// Rótulo (a dica).
        label: String,
        /// Glifo.
        icon: Symbol,
        /// Estado.
        visual: TitleBarButtonVisual,
    },
    /// Minimizar, maximizar (ou restaurar) e fechar, na `TitleBar`.
    CaptionButton(CaptionButtonVisual),
    /// `TreeView`: não pinta nada; a rolagem e as linhas são filhos. O visual serve às
    /// ferramentas.
    TreeView(TreeViewVisual),
    /// Uma linha do `TreeView`.
    TreeViewItem {
        /// Rótulo.
        label: String,
        /// Ícone.
        icon: Symbol,
        /// Selo depois do rótulo.
        badge: Option<Symbol>,
        /// Estado.
        visual: TreeViewItemVisual,
    },
    /// `DataGrid`: pinta só o fundo do arrasto e o texto de quando não há linhas; o
    /// cabeçalho, a rolagem e as linhas são filhos. O visual serve às ferramentas.
    DataGrid(DataGridVisual),
    /// O cabeçalho do `DataGrid`.
    DataGridHeader(DataGridHeaderVisual),
    /// Uma linha do `DataGrid`.
    DataGridRow(DataGridRowVisual),
    /// `PropertyGrid`: não pinta nada; a barra, as linhas e a descrição são filhos. O visual
    /// serve às ferramentas.
    PropertyGrid(PropertyGridVisual),
    /// Uma linha de propriedade da `PropertyGrid`.
    PropertyGridRow {
        /// Nome.
        name: String,
        /// Valor como a linha mostra em repouso (a senha mascarada, a `.check` vazia).
        value: String,
        /// Estado.
        visual: PropertyGridRowVisual,
    },
    /// O cabeçalho de uma categoria da `PropertyGrid`.
    PropertyGridCategory {
        /// Nome.
        label: String,
        /// Estado.
        visual: PropertyGridCategoryVisual,
    },
    /// O divisor da descrição da `PropertyGrid`.
    PropertyGridSizer(PropertyGridSizerVisual),
    /// Widget de fora do core (o `Terminal`), com o visual num tipo que o `Drawer` conhece.
    Custom(CustomLook),
}

/// Visual de um widget de fora do core. O `Drawer` reconhece o tipo concreto por
/// [`CustomVisual::as_any`]; o que ele não conhece não é medido nem pintado.
pub trait CustomVisual: Any + fmt::Debug {
    /// O visual como `Any`, para o `Drawer` reconhecer o tipo.
    fn as_any(&self) -> &dyn Any;

    /// Se `other` pinta igual: quando é igual, o `Ui` não regrava o widget.
    fn eq_visual(&self, other: &dyn CustomVisual) -> bool;

    /// O mesmo visual sem o cursor de texto, na fase apagada do pisca; `None` se não tem
    /// cursor.
    fn hide_caret(&self) -> Option<CustomLook> {
        None
    }
}

/// Visual de um widget de fora do core, compartilhado: o `Look` é clonado a cada frame, e
/// clonar só conta uma referência.
#[derive(Clone, Debug)]
pub struct CustomLook(pub Rc<dyn CustomVisual>);

impl CustomLook {
    /// Embrulha `visual`.
    pub fn new(visual: impl CustomVisual) -> Self {
        Self(Rc::new(visual))
    }

    /// O visual, se for do tipo `T`.
    pub fn downcast<T: CustomVisual>(&self) -> Option<&T> {
        self.0.as_any().downcast_ref()
    }
}

impl PartialEq for CustomLook {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || self.0.eq_visual(&*other.0)
    }
}

/// Tipo de popup, para o `Drawer` escolher o raio e o fundo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PopupKind {
    /// `Flyout`.
    Flyout,
    /// `MenuFlyout`.
    Menu,
    /// Lista do `ComboBox`.
    ComboBox,
    /// Dica.
    ToolTip,
    /// Botões de incremento do `NumberBox`, sobre o campo.
    NumberBox,
}

impl Look {
    /// Se a altura depende da largura (texto que quebra linhas, imagem que encolhe na
    /// proporção): o layout mede de novo a cada largura que oferece.
    pub fn reflows(&self) -> bool {
        matches!(
            self,
            Self::Text(TextVisual { wrap: true, .. })
                | Self::RichText(RichTextVisual { wrap: true, .. })
                | Self::InfoBar(_)
                | Self::Image(_)
                | Self::TextEditor(TextEditorVisual {
                    wrap: true,
                    max_height: Some(_),
                    ..
                })
        )
    }
}

/// Espaçamento nos quatro lados, em pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    /// Em cima.
    pub top: f64,
    /// À direita.
    pub right: f64,
    /// Embaixo.
    pub bottom: f64,
    /// À esquerda.
    pub left: f64,
}

impl Insets {
    /// Nenhum espaçamento.
    pub const ZERO: Self = Self::all(0.0);

    /// Lados um a um, na ordem do CSS.
    pub const fn new(top: f64, right: f64, bottom: f64, left: f64) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// O mesmo valor nos quatro lados.
    pub const fn all(value: f64) -> Self {
        Self::new(value, value, value, value)
    }

    /// `vertical` em cima e embaixo, `horizontal` dos lados.
    pub const fn symmetric(vertical: f64, horizontal: f64) -> Self {
        Self::new(vertical, horizontal, vertical, horizontal)
    }
}

impl std::ops::Add for Insets {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(
            self.top + other.top,
            self.right + other.right,
            self.bottom + other.bottom,
            self.left + other.left,
        )
    }
}

impl From<f64> for Insets {
    fn from(value: f64) -> Self {
        Self::all(value)
    }
}

impl From<i32> for Insets {
    fn from(value: i32) -> Self {
        Self::all(value.into())
    }
}

/// Eixo principal de um contêiner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// De cima para baixo (`Column`).
    #[default]
    Column,
    /// Da esquerda para a direita (`Row`).
    Row,
}

/// Alinhamento dos filhos no eixo cruzado.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// No começo.
    Start,
    /// No centro.
    Center,
    /// No fim.
    End,
    /// Ocupando o eixo inteiro (padrão): numa `Column`, os botões ficam da largura dela.
    #[default]
    Stretch,
}

/// Distribuição dos filhos no eixo principal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Justify {
    /// Juntos no começo (padrão).
    #[default]
    Start,
    /// Juntos no centro.
    Center,
    /// Juntos no fim.
    End,
    /// O primeiro no começo, o último no fim e o resto do espaço dividido entre eles.
    SpaceBetween,
}

/// Como um contêiner dispõe os filhos (flexbox).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flex {
    /// Eixo principal.
    pub direction: Direction,
    /// Espaço entre filhos vizinhos.
    pub gap: f64,
    /// Espaço entre a borda do contêiner e os filhos.
    pub padding: Insets,
    /// Alinhamento no eixo cruzado.
    pub align: Align,
    /// Distribuição no eixo principal.
    pub justify: Justify,
    /// Os filhos crescem para ocupar o eixo principal inteiro, como o filho de um `Form`.
    pub fill: bool,
    /// Os filhos que não cabem vão para a linha (ou coluna) seguinte, com o `gap` também
    /// entre as linhas (`Row::wrap`).
    pub wrap: bool,
    /// Grade de colunas iguais, com pelo menos essa largura cada, em quantas couberem
    /// (`UniformGrid`). Ignora `direction`, `justify` e `fill`.
    pub min_column_width: Option<f64>,
}
