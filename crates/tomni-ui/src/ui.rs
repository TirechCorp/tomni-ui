//! `Ui`: junta árvore, layout, tema e render num loop de frame.

use slotmap::SecondaryMap;
use tomni_ui_core::kurbo::{Point, Rect, Shape, Size, Vec2};
use tomni_ui_core::{
    Clipboard, Cursor, DragData, FocusSource, Ime, Insets, Key, Look, Modifiers, NativeFocus,
    NativePlace, NodeId, PointerKind, RESIZE_BORDER, ResizeEdge, TextInput, ThemeColor, Token,
    Tree, View, Widget, WindowChrome, WindowRequest,
};
use tomni_ui_layout::LayoutEngine;
use tomni_ui_reactive::{Scope, run_effects, untrack};
use tomni_ui_render::{Canvas, PATH_TOLERANCE, Recording};
use tomni_ui_theme::{Drawer, Theme};

/// Uma tela montada: recebe os eventos da plataforma, roda o frame e pinta.
///
/// Cada widget tem a sua gravação de desenho. O [`Ui::frame`] grava de novo só os widgets cujo
/// [`Look`] ou tamanho mudou (o repaint seletivo); o [`Ui::render`] reproduz as gravações na
/// ordem da árvore, deslocadas para onde o widget está agora. Mudar de lugar (pelo layout ou
/// rolando um `ScrollViewer`) não grava de novo, e o conteúdo de um `ScrollViewer` fora da
/// área visível nem é reproduzido.
///
/// ```
/// use tomni_ui::prelude::*;
/// use tomni_ui::theme::{FluentDrawer, Theme};
/// use tomni_ui::Ui;
///
/// let mut ui = Ui::new(320, 200, FluentDrawer::new(Theme::Light), || {
///     let n = signal(0);
///     Form::new("Contador").child(
///         Column::new()
///             .padding(16)
///             .child(Button::new(move || format!("Cliques: {}", n.get())).id("rotulo"))
///             .child(Button::new("+1").id("mais").on_click(move || n.update(|n| *n += 1))),
///     )
/// });
/// assert_eq!(ui.title().as_deref(), Some("Contador"));
/// assert_eq!(ui.repaints_since_last(), ["form", "column", "rotulo", "mais"]);
/// ```
pub struct Ui {
    scope: Scope,
    tree: Tree,
    layout: LayoutEngine,
    drawer: Box<dyn Drawer>,
    size: (u32, u32),
    states: SecondaryMap<NodeId, State>,
    /// Widgets gravados desde o último [`Ui::repaints_since_last`].
    repaints: Vec<NodeId>,
    /// Mede e grava tudo no próximo frame (tema novo, por exemplo).
    invalidate_all: bool,
    /// Fase do cursor piscando: `false` esconde o cursor do campo com foco.
    caret_visible: bool,
    /// Widget a gravar de novo porque o cursor piscou.
    blinked: Option<NodeId>,
    /// Se a janela do app ([`tomni_ui_core::native::host`]) existia no último frame: quando
    /// ela chega, os widgets nativos são colocados de novo e criam o controle do sistema.
    hosted: bool,
    /// Pedidos à janela feitos pelo próprio `Ui` (redimensionar pela borda).
    window_requests: Vec<WindowRequest>,
    /// Etiqueta do arrasto em andamento: o texto, o ponteiro e o desenho.
    drag_label: Option<(String, Point, Recording)>,
}

/// Distância da etiqueta do arrasto ao ponteiro, para a direita e para baixo: o ponteiro não
/// cobre o texto.
const DRAG_LABEL_OFFSET: Vec2 = Vec2::new(8.0, 12.0);

/// O que o `Ui` guarda de cada widget.
#[derive(Debug, Default)]
struct State {
    /// `Look` gravado; `None` antes da primeira gravação.
    painted: Option<Look>,
    /// `.visible()` no último frame, sem olhar os ancestrais.
    visible: bool,
    /// Desenho do widget, feito com o retângulo de layout `origin` e tamanho `size`. Num
    /// lugar novo (rolado, ou movido pelo layout), é reproduzido deslocado.
    recording: Recording,
    origin: Point,
    size: Size,
    /// [`Widget::dynamic_width`](tomni_ui_core::Widget::dynamic_width) aplicada ao layout.
    width: Option<f64>,
    /// [`Widget::dynamic_basis`](tomni_ui_core::Widget::dynamic_basis) aplicada ao layout.
    basis: Option<(f64, f64)>,
    /// [`Widget::dynamic_grow`](tomni_ui_core::Widget::dynamic_grow) aplicada ao layout.
    grow: Option<f64>,
    /// [`Widget::opacity`](tomni_ui_core::Widget::opacity): o widget e os descendentes são
    /// pintados com ela.
    opacity: f64,
    /// Se o widget põe um controle nativo por cima ([`Widget::native`]).
    native: bool,
    /// O último [`Widget::place_native`].
    placed: Option<NativePlace>,
}

impl Ui {
    /// Roda `view` uma vez, monta a árvore e faz o primeiro frame numa janela de
    /// `width` × `height` pixels.
    pub fn new<V: View>(
        width: u32,
        height: u32,
        drawer: impl Drawer + 'static,
        view: impl FnOnce() -> V,
    ) -> Self {
        Self::new_boxed(width, height, drawer, || Box::new(view()))
    }

    /// Como [`Ui::new`], com a raiz já numa caixa: a tela de uma janela aberta por
    /// [`open_window`](tomni_ui_core::open_window).
    pub fn new_boxed(
        width: u32,
        height: u32,
        drawer: impl Drawer + 'static,
        view: impl FnOnce() -> Box<dyn Widget>,
    ) -> Self {
        let scope = Scope::root();
        let root = scope.run(view);
        let mut tree = Tree::mount(root, &scope);
        tree.set_text_metrics(drawer.text_metrics());
        let mut layout = LayoutEngine::new(&tree);
        layout.set_viewport(Size::new(f64::from(width), f64::from(height)));
        let mut states = SecondaryMap::new();
        for id in tree.ids() {
            states.insert(id, State::new(&tree, id));
        }
        let mut ui = Self {
            scope,
            tree,
            layout,
            drawer: Box::new(drawer),
            size: (width, height),
            states,
            repaints: Vec::new(),
            invalidate_all: true,
            caret_visible: true,
            blinked: None,
            hosted: false,
            window_requests: Vec::new(),
            drag_label: None,
        };
        ui.frame();
        ui
    }

    /// Árvore montada.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// Muda o widget em `id` de fora de um evento, como [`Tree::with_widget_mut`].
    pub fn with_widget_mut<R>(&mut self, id: NodeId, f: impl FnOnce(&mut dyn Widget) -> R) -> R {
        self.tree.with_widget_mut(id, f)
    }

    /// `Drawer` atual.
    pub fn drawer(&self) -> &dyn Drawer {
        &*self.drawer
    }

    pub(crate) fn drawer_mut(&mut self) -> &mut dyn Drawer {
        &mut *self.drawer
    }

    /// Troca o `Drawer` (o tema): tudo é medido e gravado de novo no próximo frame.
    pub fn set_drawer(&mut self, drawer: impl Drawer + 'static) {
        self.tree.set_text_metrics(drawer.text_metrics());
        self.drawer = Box::new(drawer);
        self.invalidate_all = true;
    }

    /// Mede e grava tudo de novo no próximo frame (o `Drawer` mudou por dentro).
    pub(crate) fn invalidate(&mut self) {
        self.invalidate_all = true;
    }

    /// Largura e altura da janela.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Muda o tamanho da janela.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        self.layout
            .set_viewport(Size::new(f64::from(width), f64::from(height)));
    }

    /// Área ocupada pelo sistema (barra de status, entalhe), descontada da tela.
    pub fn set_safe_area(&mut self, insets: Insets) {
        self.layout.set_safe_area(insets);
    }

    /// Título da janela, lido do `Form` raiz.
    pub fn title(&self) -> Option<String> {
        self.tree.widget(self.tree.root()).title()
    }

    /// Procura o widget com o id `name`.
    pub fn find(&self, name: &str) -> Option<NodeId> {
        self.tree.find(name)
    }

    /// Procura o link `name` (`<id>-link-<n>`) de um `RichText`, ou o botão de ação
    /// (`<id>-acao`) de uma `InfoBar`: o widget dono e a área do link (a da primeira linha, se
    /// ele quebra) ou do botão, na janela.
    pub fn find_link(&self, name: &str) -> Option<(NodeId, Rect)> {
        let tree = &self.tree;
        for id in tree.ids() {
            let widget = tree.widget(id);
            let kind = widget.kind();
            if kind != "rich_text" && kind != "info_bar" {
                continue;
            }
            let Some(n) = untrack(|| widget.part_ids()).iter().position(|l| l == name) else {
                continue;
            };
            drop(widget);
            let rect = tree.rect(id);
            match self.look(id) {
                Look::RichText(visual) => {
                    let first = *self.drawer.rich_text_links(rect, &visual).get(n)?.first()?;
                    return Some((id, first + rect.origin().to_vec2()));
                }
                // O botão de ação da `InfoBar`.
                Look::InfoBar(visual) => {
                    let button = self.drawer.info_bar_action(rect, &visual)?;
                    return Some((id, button + rect.origin().to_vec2()));
                }
                _ => continue,
            }
        }
        None
    }

    /// Retângulo do widget `id` na janela.
    pub fn rect(&self, id: NodeId) -> Rect {
        self.tree.rect(id)
    }

    /// Parte do widget `id` que recebe o ponteiro: quase sempre o retângulo inteiro; na
    /// `InfoBar`, o botão de fechar.
    pub fn hit_rect(&self, id: NodeId) -> Rect {
        self.tree.hit_rect(id)
    }

    /// A parte com o texto `label` que o grupo `group` montou: o item da lista de um
    /// `ComboBox` ou de um `AutoSuggestBox`, a opção de um `RadioButtons`, a aba de um
    /// `TabView` ou o item de um `NavigationView`. Sem ela, devolve os textos das partes que
    /// existem, na ordem.
    pub fn part(&self, group: NodeId, label: &str) -> Result<NodeId, Vec<String>> {
        let tree = &self.tree;
        let parts: Vec<(NodeId, String)> = tree
            .ids()
            .filter(|&n| tree.widget(n).common().is_generated())
            // Os botões da faixa de abas não são partes que se escolhem pelo rótulo.
            .filter(|&n| tree.widget(n).kind() != "tab_button")
            .filter(|&n| {
                std::iter::successors(tree.parent(n), |&p| tree.parent(p))
                    .find(|&p| tree.widget(p).names_children())
                    == Some(group)
            })
            .filter_map(|n| untrack(|| tree.widget(n).text()).map(|text| (n, text)))
            .collect();
        // Com duas partes de mesmo texto (o item do `NavigationView` e a cópia dele no
        // flyout do modo compacto), vale a que aparece.
        let mut matching = parts.iter().filter(|(_, text)| text == label);
        let first = matching.clone().next().map(|&(n, _)| n);
        matching
            .find(|&&(n, _)| tree.shown(n))
            .map(|&(n, _)| n)
            .or(first)
            .ok_or_else(|| parts.into_iter().map(|(_, text)| text).collect())
    }

    /// O próximo clique para escolher o item de rótulo `label` do `NavigationView` `nav`, como
    /// uma pessoa faria: o próprio item, se aparece (com `true`); senão, o grupo fechado em
    /// volta dele ou, com o painel fechado no modo mínimo, o ☰ (com `false`). Sem o item,
    /// devolve os rótulos dos itens que existem, na ordem.
    pub fn navigation_step(&self, nav: NodeId, label: &str) -> Result<(NodeId, bool), Vec<String>> {
        let tree = &self.tree;
        let inside = |n: NodeId| {
            std::iter::successors(tree.parent(n), |&p| tree.parent(p)).any(|p| p == nav)
        };
        let items: Vec<(NodeId, String)> = tree
            .ids()
            .filter(|&n| tree.widget(n).kind() == "navigation_item" && inside(n))
            .filter_map(|n| untrack(|| tree.widget(n).text()).map(|text| (n, text)))
            .collect();
        let matching: Vec<NodeId> = items
            .iter()
            .filter(|(_, text)| text == label)
            .map(|&(n, _)| n)
            .collect();
        if matching.is_empty() {
            let mut labels: Vec<String> = Vec::new();
            for (_, text) in items {
                if !labels.contains(&text) {
                    labels.push(text);
                }
            }
            return Err(labels);
        }
        if let Some(&item) = matching.iter().find(|&&n| tree.shown(n)) {
            return Ok((item, true));
        }
        // O grupo que aparece mais perto do item: o de dentro do popup do painel compacto,
        // quando ele está aberto, fica mais fundo que o do painel.
        let step = matching
            .iter()
            .filter_map(|&n| {
                std::iter::successors(tree.parent(n), |&p| tree.parent(p))
                    .take_while(|&p| p != nav)
                    .find(|&p| tree.widget(p).kind() == "navigation_item" && tree.shown(p))
            })
            .max_by_key(|&p| std::iter::successors(Some(p), |&a| tree.parent(a)).count());
        if let Some(group) = step {
            return Ok((group, false));
        }
        // O ☰ do próprio `NavigationView` ou, ligado à `TitleBar`, o da barra.
        let toggles: Vec<NodeId> = tree
            .ids()
            .filter(|&n| tree.widget(n).kind() == "navigation_toggle" && tree.shown(n))
            .collect();
        let toggle = toggles
            .iter()
            .find(|&&n| inside(n))
            .or(toggles.first())
            .copied();
        Ok((toggle.unwrap_or(matching[0]), false))
    }

    /// Ponto do trilho do `Slider` `id` que corresponde a `value` (levado para dentro da
    /// faixa), na altura do polegar; `None` se `id` não for um `Slider`.
    pub fn slider_point(&self, id: NodeId, value: f64) -> Option<Point> {
        let (min, max) = untrack(|| self.tree.widget(id).range())?.into_inner();
        let rect = self.tree.rect(id);
        let fraction = if max > min {
            ((value - min) / (max - min)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let half = tomni_ui_core::SLIDER_THUMB_SIZE / 2.0;
        let x = rect.x0 + half + fraction * (rect.width() - 2.0 * half);
        Some(Point::new(x, self.tree.hit_rect(id).center().y))
    }

    /// Ponteiro em `point`.
    pub fn pointer_move(&mut self, point: Point, kind: PointerKind) {
        untrack(|| self.tree.pointer_move(point, kind));
    }

    /// Botão primário ou dedo pressionado em `point`. `clicks` é 1 no clique simples, 2 no
    /// duplo e 3 no triplo; quem conta é a plataforma, com o intervalo do sistema.
    ///
    /// Na faixa de [`RESIZE_BORDER`] px da borda de uma janela sem moldura
    /// ([`Ui::resize_edge`]), o mouse pede para redimensionar a janela, e os widgets não
    /// recebem o clique.
    pub fn pointer_down(&mut self, point: Point, kind: PointerKind, clicks: u8) {
        if kind == PointerKind::Mouse
            && let Some(edge) = self.resize_edge(point)
        {
            self.window_requests.push(WindowRequest::Resize(edge));
            return;
        }
        untrack(|| self.tree.pointer_down(point, kind, clicks));
    }

    /// Botão secundário (o direito) pressionado em `point`: o `Terminal` cola.
    pub fn secondary_down(&mut self, point: Point) {
        untrack(|| self.tree.secondary_down(point));
    }

    /// Botão secundário (o direito) solto em `point`: avisa quem usou o pressionar (o
    /// `Terminal` que passa o mouse ao programa).
    pub fn secondary_up(&mut self, point: Point) {
        untrack(|| self.tree.secondary_up(point));
    }

    /// Botão do meio pressionado em `point`.
    pub fn middle_down(&mut self, point: Point) {
        untrack(|| self.tree.middle_down(point));
    }

    /// Arquivos do sistema soltos em `point`. Devolve se algum alvo aceitou.
    pub fn files_dropped(&mut self, paths: Vec<std::path::PathBuf>, point: Point) -> bool {
        untrack(|| self.tree.files_dropped(paths, Some(point)))
    }

    /// Botão do meio solto em `point`: o clique do meio fecha uma aba.
    pub fn middle_up(&mut self, point: Point) {
        untrack(|| self.tree.middle_up(point));
    }

    /// Botão primário ou dedo solto em `point`.
    pub fn pointer_up(&mut self, point: Point, kind: PointerKind) {
        untrack(|| self.tree.pointer_up(point, kind));
    }

    /// O ponteiro saiu da janela ou o gesto foi cancelado.
    pub fn pointer_cancel(&mut self) {
        untrack(|| self.tree.pointer_cancel());
    }

    /// O item que está sendo arrastado a partir desta janela, depois de passar do limiar: a
    /// plataforma o leva às outras janelas ([`Ui::foreign_drag_over`]).
    pub fn drag_data(&self) -> Option<DragData> {
        self.tree.drag_data()
    }

    /// Um item arrastado a partir de outra janela passou por `point` desta. Devolve se algum
    /// alvo aceita.
    pub fn foreign_drag_over(&mut self, data: &DragData, point: Point) -> bool {
        untrack(|| self.tree.foreign_drag_over(data, point))
    }

    /// O item arrastado a partir de outra janela saiu desta.
    pub fn foreign_drag_leave(&mut self) {
        untrack(|| self.tree.foreign_drag_leave());
    }

    /// Um item arrastado a partir de outra janela foi solto em `point` desta. Devolve se algum
    /// alvo aceitou.
    pub fn foreign_drop(&mut self, data: DragData, point: Point) -> bool {
        untrack(|| self.tree.foreign_drop(data, point))
    }

    /// Cancela o arrasto que começou nesta janela, sem soltar em lugar nenhum.
    pub fn cancel_drag(&mut self) {
        untrack(|| self.tree.cancel_drag());
    }

    /// O arrasto que começou nesta janela foi solto fora de todas as janelas, em `at` na tela
    /// (`None` onde o sistema não informa). Devolve se havia arrasto.
    pub fn drop_outside(&mut self, at: Option<(i32, i32)>) -> bool {
        untrack(|| self.tree.drop_outside(at))
    }

    /// Roda do mouse ou rolagem do touchpad em `point`, em pixels: `delta` positivo mostra o
    /// que está mais abaixo (ou à direita). Rola o `ScrollViewer` mais interno sob o ponteiro
    /// que ainda anda nessa direção. Devolve `true` se algo rolou.
    pub fn wheel(&mut self, point: Point, delta: Vec2) -> bool {
        untrack(|| self.tree.wheel(point, delta))
    }

    /// Rola o `ScrollViewer` `id` em `delta` pixels, como a roda sobre ele. Devolve `true` se
    /// mudou.
    pub fn scroll_by(&mut self, id: NodeId, delta: Vec2) -> bool {
        untrack(|| self.tree.scroll_by(id, delta))
    }

    /// Rola o `ScrollViewer` `id` até `offset` (limitado ao que dá para rolar). Devolve `true`
    /// se mudou.
    pub fn scroll_to(&mut self, id: NodeId, offset: Vec2) -> bool {
        untrack(|| self.tree.scroll_to(id, offset))
    }

    /// Rola os `ScrollViewer` em volta de `id` até ele aparecer inteiro.
    pub fn bring_into_view(&mut self, id: NodeId) {
        untrack(|| self.tree.bring_into_view(id));
    }

    // Os eventos leem signals (`disabled`, o texto) só para decidir o que fazer, sem criar
    // dependência: daí o `untrack`, que também cala o aviso do `reactive_graph` em debug.

    /// Tecla pressionada. Tab move o foco.
    pub fn key_down(&mut self, key: Key) {
        untrack(|| self.tree.key_down(key));
    }

    /// Tecla solta.
    pub fn key_up(&mut self, key: Key) {
        untrack(|| self.tree.key_up(key));
    }

    /// Shift+Tab: foco no widget anterior.
    pub fn focus_prev(&mut self) {
        untrack(|| self.tree.focus_prev());
    }

    /// Modificadores pressionados agora. A plataforma avisa quando mudam; os atalhos
    /// (`Ctrl+C`) chegam como [`Key::Char`] com os modificadores.
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.tree.set_modifiers(modifiers);
    }

    /// Texto digitado, já com o layout do teclado e as teclas mortas aplicados.
    pub fn type_text(&mut self, text: &str) {
        untrack(|| self.tree.text(text));
    }

    /// Composição do IME.
    pub fn ime(&mut self, ime: Ime) {
        untrack(|| self.tree.ime(ime));
    }

    /// Área de transferência usada pelos widgets.
    pub fn clipboard(&mut self) -> &mut dyn Clipboard {
        self.tree.clipboard_mut()
    }

    /// Troca a área de transferência (a do sistema, na plataforma).
    pub fn set_clipboard(&mut self, clipboard: impl Clipboard + 'static) {
        self.tree.set_clipboard(Box::new(clipboard));
    }

    /// Entrada de texto pedida pelo widget com foco: com `Some`, a plataforma liga o IME (e,
    /// no Android, abre o teclado virtual) e põe a janela de candidatos junto do cursor; com
    /// `None`, desliga. Consultar depois de cada frame.
    pub fn text_input(&mut self) -> Option<TextInput> {
        let id = self.tree.focused()?;
        let purpose = self.tree.widget(id).text_input()?;
        let rect = self.tree.rect(id);
        let area = match self.tree.look(id) {
            Look::Edit(visual) => self.drawer.caret_rect(rect, &visual),
            Look::TextEditor(visual) => self.drawer.text_editor_caret_rect(rect, &visual),
            Look::Custom(custom) => self.drawer.custom_caret_rect(rect, &custom),
            _ => None,
        };
        Some(TextInput {
            area: area.unwrap_or(rect),
            purpose,
        })
    }

    /// Mostra ou esconde o cursor do campo com foco. A plataforma alterna a cada 530 ms
    /// enquanto há [`Ui::text_input`] e volta a mostrar a cada tecla ou clique; o `TestApp`
    /// nunca esconde.
    pub fn set_caret_visible(&mut self, visible: bool) {
        if self.caret_visible != visible {
            self.caret_visible = visible;
            self.blinked = self.tree.focused();
        }
    }

    /// `Look` com que o widget é pintado: o da árvore, com o cursor escondido na fase apagada
    /// do pisca. Um `Text` ou `Icon` sem cor própria herda a do ancestral mais próximo que define uma (o
    /// card pressionado ou desabilitado), como o `Foreground` do WinUI; dentro de um card
    /// desabilitado, as cores de texto do tema (`secondary`) também viram a desabilitada.
    pub fn look(&self, id: NodeId) -> Look {
        let mut look = self.tree.look(id);
        match &mut look {
            Look::Edit(visual) if !self.caret_visible => visual.caret = None,
            Look::TextEditor(visual) if !self.caret_visible => visual.caret = None,
            Look::Custom(custom) if !self.caret_visible => {
                if let Some(hidden) = custom.0.hide_caret() {
                    *custom = hidden;
                }
            }
            Look::Text(visual) => {
                let inherited = untrack(|| self.inherited_foreground(id));
                inherit(&mut visual.color, inherited);
            }
            Look::RichText(visual) => {
                let inherited = untrack(|| self.inherited_foreground(id));
                inherit(&mut visual.color, inherited);
            }
            Look::Icon(visual) => {
                let inherited = untrack(|| self.inherited_foreground(id));
                inherit(&mut visual.color, inherited);
            }
            _ => {}
        }
        look
    }

    /// Cor de texto que os ancestrais de `id` passam aos descendentes.
    fn inherited_foreground(&self, id: NodeId) -> Option<ThemeColor> {
        let mut parent = self.tree.parent(id);
        while let Some(ancestor) = parent {
            if let Some(color) = self.tree.widget(ancestor).foreground() {
                return Some(color);
            }
            parent = self.tree.parent(ancestor);
        }
        None
    }

    /// Acrescenta a `ids` os descendentes dos widgets que passam a cor de texto adiante: o
    /// `Look` deles muda junto com o do ancestral.
    fn with_foreground_descendants(&self, ids: &mut Vec<NodeId>) {
        let mut pending: Vec<NodeId> = ids
            .iter()
            .copied()
            .filter(|&id| self.tree.contains(id) && self.tree.widget(id).passes_foreground())
            .collect();
        while let Some(id) = pending.pop() {
            for &child in self.tree.children(id) {
                ids.push(child);
                pending.push(child);
            }
        }
    }

    /// Roda os efeitos, monta e desmonta o que os `Show` e `For` pedirem, refaz o layout se
    /// preciso e grava de novo os widgets cujo visual ou tamanho mudou. Retorna se a tela
    /// precisa ser pintada de novo.
    pub fn frame(&mut self) -> bool {
        let mut changed = Vec::new();
        // As respostas dos seletores de arquivo chegam aqui, nunca dentro de um handler.
        tomni_ui_core::dialogs::deliver();
        // Os cliques na bandeja do sistema também.
        tomni_ui_core::tray::deliver();
        tomni_ui_core::clipboard::flush(self.tree.clipboard_mut());
        loop {
            run_effects();
            untrack(|| self.tree.sync_focus());
            let changes = untrack(|| self.tree.reconcile());
            if changes.is_empty() {
                break;
            }
            self.layout.sync(&self.tree, &changes);
            for &id in &changes.removed {
                self.states.remove(id);
            }
            for &id in &changes.added {
                self.states.insert(id, State::new(&self.tree, id));
                changed.push(id);
            }
        }

        changed.extend(self.tree.take_dirty());
        changed.extend(self.blinked.take());
        if std::mem::take(&mut self.invalidate_all) {
            changed = self.tree.ids().collect();
            for (_, state) in &mut self.states {
                state.painted = None;
            }
        }
        let mut repaint = false;
        let mut record = Vec::new();
        // Medir pode mudar o layout, e o layout muda a rolagem (e o visual) dos
        // `ScrollViewer`: repete até assentar.
        for _ in 0..6 {
            repaint |= self.update_looks(changed, &mut record);
            if self.layout.needs_layout() {
                let drawer = &mut self.drawer;
                let states = &self.states;
                let moved = self.layout.compute(&mut self.tree, |id, width| {
                    let look = states[id]
                        .painted
                        .as_ref()
                        .expect("toda folha é medida antes do layout");
                    drawer.measure_wrapped(look, width)
                });
                repaint |= !moved.is_empty();
                for id in moved {
                    // Um fragmento não desenha nada: o retângulo dele é só o dos filhos.
                    if self.tree.widget(id).fragment() {
                        continue;
                    }
                    if self.states[id].size != self.tree.layout_rect(id).size() {
                        record.push(id);
                    } else if self.on_screen(id) {
                        // Só mudou de lugar: a gravação vale, deslocada, mas a tela muda.
                        self.repaints.push(id);
                    }
                }
            }
            // Uma camada que abriu já tem lugar: rola até o widget que ela focou.
            untrack(|| self.tree.reveal_pending());
            // Os widgets que medem o próprio conteúdo (o `TextEditor`) sabem onde ficaram.
            untrack(|| self.tree.place_widgets());
            self.place_natives();
            // Um widget que soube do próprio tamanho ([`Widget::resized`]) pode ter mudado
            // signals: os efeitos põem na fila quem depende deles.
            run_effects();
            untrack(|| self.tree.sync_focus());
            changed = self.tree.take_dirty();
            if changed.is_empty() {
                break;
            }
        }
        record.sort_unstable_by_key(|&id| self.tree.position(id));
        record.dedup();

        repaint |= !record.is_empty();
        for id in record {
            let rect = self.tree.layout_rect(id);
            let state = &mut self.states[id];
            let look = state
                .painted
                .as_ref()
                .expect("todo widget é medido antes de ser gravado");
            let mut recording = Recording::new(self.size.0, self.size.1);
            self.drawer.draw(&mut recording, rect, look);
            state.recording = recording;
            state.origin = rect.origin();
            state.size = rect.size();
            self.repaints.push(id);
        }
        repaint | self.record_drag_label()
    }

    /// Grava de novo a etiqueta do arrasto se o texto ou o ponteiro mudaram. Devolve se mudou.
    fn record_drag_label(&mut self) -> bool {
        let now = self.tree.drag_label();
        let before = self
            .drag_label
            .as_ref()
            .map(|(label, at, _)| (label.as_str(), *at));
        if now.as_ref().map(|(label, at)| (label.as_str(), *at)) == before {
            return false;
        }
        self.drag_label = now.map(|(label, at)| {
            let look = Look::DragLabel(label.clone());
            let size = self.drawer.measure_wrapped(&look, None);
            let (width, height) = (f64::from(self.size.0), f64::from(self.size.1));
            let mut origin = at + DRAG_LABEL_OFFSET;
            // Perto da borda direita ou de baixo, a etiqueta fica do outro lado do ponteiro.
            if origin.x + size.width > width {
                origin.x = (at.x - DRAG_LABEL_OFFSET.x - size.width).max(0.0);
            }
            if origin.y + size.height > height {
                origin.y = (at.y - DRAG_LABEL_OFFSET.y - size.height).max(0.0);
            }
            let rect = Rect::from_origin_size(origin, size);
            let mut recording = Recording::new(self.size.0, self.size.1);
            self.drawer.draw(&mut recording, rect, &look);
            (label, at, recording)
        });
        true
    }

    /// Se a tela tem uma [`TitleBar`](tomni_ui_core::TitleBar): a plataforma tira a moldura
    /// do sistema.
    pub fn has_title_bar(&self) -> bool {
        self.tree.window_state().is_some()
    }

    /// A janela ganhou (`true`) ou perdeu o foco do sistema: a `TitleBar` apaga quando inativa.
    pub fn set_window_active(&mut self, active: bool) {
        if let Some(state) = self.tree.window_state() {
            state.set_active(active);
        }
    }

    /// A janela foi maximizada (`true`) ou restaurada: muda o glifo do maximizar e desliga o
    /// redimensionar pela borda.
    pub fn set_maximized(&mut self, maximized: bool) {
        if let Some(state) = self.tree.window_state() {
            state.set_maximized(maximized);
        }
    }

    /// Se a janela está maximizada, como a plataforma informou ([`Ui::set_maximized`]).
    pub fn is_maximized(&self) -> bool {
        self.tree
            .window_state()
            .is_some_and(|state| untrack(|| state.maximized()))
    }

    /// Quem desenha a moldura da janela com a `TitleBar` (a plataforma decide ao abrir).
    pub fn set_chrome(&mut self, chrome: WindowChrome) {
        if let Some(state) = self.tree.window_state() {
            state.set_chrome(chrome);
        }
    }

    /// Pedidos à janela (arrastar, maximizar, fechar, redimensionar) desde a última chamada,
    /// na ordem. A plataforma atende depois de cada evento.
    pub fn take_window_requests(&mut self) -> Vec<WindowRequest> {
        let mut requests = std::mem::take(&mut self.window_requests);
        requests.extend(self.tree.take_window_requests());
        requests
    }

    /// Forma do ponteiro do mouse em `point` ([`Tree::cursor`]).
    pub fn cursor(&self, point: Point) -> Cursor {
        self.tree.cursor(point)
    }

    /// A borda (ou o canto) da janela em `point` que redimensiona: só com a `TitleBar`, a
    /// moldura do app ([`WindowChrome::Custom`]) e a janela fora do maximizado.
    pub fn resize_edge(&self, point: Point) -> Option<ResizeEdge> {
        let state = self.tree.window_state()?;
        if untrack(|| state.chrome() != WindowChrome::Custom || state.maximized()) {
            return None;
        }
        let (width, height) = (f64::from(self.size.0), f64::from(self.size.1));
        let west = point.x < RESIZE_BORDER;
        let east = point.x >= width - RESIZE_BORDER;
        let north = point.y < RESIZE_BORDER;
        let south = point.y >= height - RESIZE_BORDER;
        Some(match (north, south, west, east) {
            (true, _, true, _) => ResizeEdge::NorthWest,
            (true, _, _, true) => ResizeEdge::NorthEast,
            (_, true, true, _) => ResizeEdge::SouthWest,
            (_, true, _, true) => ResizeEdge::SouthEast,
            (true, ..) => ResizeEdge::North,
            (_, true, ..) => ResizeEdge::South,
            (.., true, _) => ResizeEdge::West,
            (.., true) => ResizeEdge::East,
            _ => return None,
        })
    }

    /// Diz aos widgets nativos onde eles aparecem e atende os pedidos de foco deles.
    fn place_natives(&mut self) {
        let natives: Vec<NodeId> = self
            .states
            .iter()
            .filter(|(_, state)| state.native)
            .map(|(id, _)| id)
            .collect();
        if natives.is_empty() {
            return;
        }
        let dark = self.drawer.tokens().theme == Theme::Dark;
        let layers = self.tree.open_layers();
        let hosted = tomni_ui_core::native::host().is_some();
        let arrived = hosted && !std::mem::replace(&mut self.hosted, hosted);
        for id in natives {
            let place = self.native_place(id, &layers, dark);
            if arrived || self.states[id].placed != Some(place) {
                self.states[id].placed = Some(place);
                self.tree
                    .with_widget_mut(id, |widget| widget.place_native(place));
            }
            let request = untrack(|| self.tree.widget(id).take_native_focus());
            let Some(request) = request else { continue };
            untrack(|| {
                if self.tree.focused() != Some(id) {
                    self.tree.set_focus(Some(id), FocusSource::Pointer);
                }
                match request {
                    NativeFocus::Took => {}
                    NativeFocus::Next => self.tree.focus_next(),
                    NativeFocus::Prev => self.tree.focus_prev(),
                }
            });
        }
    }

    /// Onde o widget nativo `id` aparece: o retângulo dele, se aparece, e se uma camada aberta
    /// acima dele, um irmão `raised` ou o corte de um `ScrollViewer` o cobre em parte.
    fn native_place(&self, id: NodeId, layers: &[NodeId], dark: bool) -> NativePlace {
        let hidden = NativePlace {
            dark,
            ..NativePlace::HIDDEN
        };
        let rect = self.tree.rect(id);
        let visible = rect.intersect(self.tree.clip(id));
        if !self.tree.shown(id) || visible.width() <= 0.0 || visible.height() <= 0.0 {
            return hidden;
        }
        let mut covered = visible != rect;
        // As camadas abertas depois da camada do widget (todas, fora de uma camada).
        let own = self.tree.layer_of(id);
        let above = own
            .and_then(|own| layers.iter().position(|&layer| layer == own))
            .map_or(0, |i| i + 1);
        covered |= layers[above.min(layers.len())..].iter().any(|&layer| {
            self.tree.layer(layer).is_some_and(|l| l.modal) || overlaps(self.tree.rect(layer), rect)
        });
        // Os irmãos `raised` de um ancestral, pintados depois dele.
        let mut node = id;
        while !covered && let Some(parent) = self.tree.parent(node) {
            if self.tree.layer(node).is_some() {
                break;
            }
            if !self.tree.widget(node).raised() {
                covered = self.tree.children(parent).iter().any(|&sibling| {
                    sibling != node
                        && self.tree.widget(sibling).raised()
                        && self.tree.shown(sibling)
                        && overlaps(self.tree.rect(sibling), rect)
                });
            }
            node = parent;
        }
        NativePlace {
            rect: Some(rect),
            covered,
            dark,
        }
    }

    /// Se `id` aparece e alguma parte dele está na área visível dos `ScrollViewer` em volta.
    fn on_screen(&self, id: NodeId) -> bool {
        let rect = self.tree.rect(id).inflate(CULL_MARGIN, CULL_MARGIN);
        self.tree.shown(id) && overlaps(rect, self.tree.clip(id))
    }

    /// Confere a visibilidade e o `Look` de `ids`, mede de novo as folhas que mudaram e põe em
    /// `record` quem precisa ser gravado. Retorna se algum widget apareceu ou sumiu.
    fn update_looks(&mut self, mut ids: Vec<NodeId>, record: &mut Vec<NodeId>) -> bool {
        ids.retain(|&id| self.tree.contains(id));
        self.with_foreground_descendants(&mut ids);
        ids.sort_unstable_by_key(|&id| self.tree.position(id));
        ids.dedup();
        let mut toggled = false;
        for id in ids {
            let visible = self.tree.visible(id);
            if self.states[id].visible != visible {
                self.states[id].visible = visible;
                self.layout.set_visible(id, visible);
                toggled = true;
            }
            let opacity = untrack(|| self.tree.widget(id).opacity());
            if self.states[id].opacity != opacity {
                self.states[id].opacity = opacity;
                toggled = true;
            }
            let width = untrack(|| self.tree.widget(id).dynamic_width());
            if self.states[id].width != width {
                self.states[id].width = width;
                let fallback = self.tree.widget(id).common().width;
                self.layout.set_width(id, width, fallback);
            }
            let basis = untrack(|| self.tree.widget(id).dynamic_basis());
            if self.states[id].basis != basis {
                self.states[id].basis = basis;
                if let Some(basis) = basis {
                    self.layout.set_basis(&self.tree, id, basis);
                }
            }
            let grow = untrack(|| self.tree.widget(id).dynamic_grow());
            if self.states[id].grow != grow {
                self.states[id].grow = grow;
                let fallback = self.tree.widget(id).common().grow;
                self.layout.set_grow(id, grow, fallback);
            }
            let look = self.look(id);
            if self.states[id].painted.as_ref() == Some(&look) {
                continue;
            }
            let widget = self.tree.widget(id);
            let fragment = widget.fragment();
            let leaf = widget.flex().is_none() && widget.scroller().is_none() && !fragment;
            let measured = widget.measured();
            drop(widget);
            if leaf {
                if look.reflows() {
                    self.layout.set_reflow(id);
                } else {
                    self.layout.set_measure(id, self.drawer.measure(&look));
                }
            } else if measured {
                self.layout.set_min_size(id, self.drawer.measure(&look));
            }
            self.states[id].painted = Some(look);
            // Um fragmento não desenha nada.
            if !fragment {
                record.push(id);
            }
        }
        toggled
    }

    /// Passa `dt` de tempo: a dica de um widget abre depois de
    /// [`TOOLTIP_DELAY`](tomni_ui_core::TOOLTIP_DELAY) com o ponteiro sobre ele. A plataforma
    /// chama enquanto há [`Ui::deadline`]; rode o [`Ui::frame`] depois.
    pub fn advance(&mut self, dt: std::time::Duration) {
        untrack(|| self.tree.advance(dt));
    }

    /// Quanto falta para uma dica abrir, se houver uma em contagem.
    pub fn deadline(&self) -> Option<std::time::Duration> {
        if tomni_ui_core::dialogs::has_answer() || tomni_ui_core::tray::has_event() {
            return Some(std::time::Duration::ZERO);
        }
        self.tree.deadline()
    }

    /// O que o [`Ui::deadline`] espera, em texto, para o rastro dos quadros
    /// (`TOMNI_UI_TRACE_FRAMES`) e o aviso da janela que pinta sem parar.
    #[doc(hidden)]
    pub fn deadline_reason(&self) -> Option<String> {
        if tomni_ui_core::dialogs::has_answer() {
            return Some("resposta de diálogo".into());
        }
        if tomni_ui_core::tray::has_event() {
            return Some("evento da bandeja".into());
        }
        self.tree.deadline_reason()
    }

    /// Pinta a tela em `canvas`, reproduzindo as gravações na ordem da árvore. Widgets ocultos
    /// (eles ou um ancestral com `.visible(false)`) ficam de fora, e o conteúdo de um
    /// `ScrollViewer` é cortado na área visível dele, com as barras por cima.
    ///
    /// Os popups abertos vêm depois, por cima de tudo, na ordem em que abriram; antes de um
    /// `ContentDialog`, a janela escurece. Por último, a etiqueta do arrasto em andamento.
    pub fn render(&self, canvas: &mut dyn Canvas) {
        self.render_node(canvas, self.tree.root(), None);
        let (width, height) = self.size;
        let window = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        for layer in self.tree.open_layers() {
            if self.tree.layer(layer).is_some_and(|l| l.modal) {
                self.drawer.draw_smoke(canvas, window);
            }
            self.render_node(canvas, layer, None);
        }
        if let Some((_, _, recording)) = &self.drag_label {
            recording.replay_at(canvas, Vec2::ZERO);
        }
    }

    /// Pinta `id` e os descendentes. `clip` é a área visível do `ScrollViewer` em volta.
    fn render_node(&self, canvas: &mut dyn Canvas, id: NodeId, clip: Option<Rect>) {
        let state = &self.states[id];
        if !state.visible {
            return;
        }
        let rect = self.tree.rect(id);
        // Fora da área visível não é pintado. A folga cobre o anel de foco e as sombras, que
        // passam um pouco do retângulo.
        if let Some(clip) = clip
            && !overlaps(rect.inflate(CULL_MARGIN, CULL_MARGIN), clip)
        {
            return;
        }
        let offset = rect.origin() - state.origin;
        let faded = state.opacity < 1.0;
        if faded {
            canvas.push_opacity(state.opacity as f32);
        }
        self.render_contents(canvas, id, rect, offset, clip);
        if faded {
            canvas.pop_opacity();
        }
    }

    /// A gravação de `id` e os descendentes, sem a opacidade.
    fn render_contents(
        &self,
        canvas: &mut dyn Canvas,
        id: NodeId,
        rect: Rect,
        offset: Vec2,
        clip: Option<Rect>,
    ) {
        let state = &self.states[id];
        // As camadas são pintadas no fim, por cima de tudo; os filhos que ficam por cima dos
        // irmãos, depois deles.
        let children = self.tree.stacked(id);
        if self.tree.widget(id).scroller().is_none() {
            state.recording.replay_at(canvas, offset);
            for child in children {
                self.render_node(canvas, child, clip);
            }
            return;
        }
        let view = clip.map_or(rect, |clip| clip.intersect(rect));
        canvas.push_clip(&rect.to_path(PATH_TOLERANCE));
        for child in children {
            self.render_node(canvas, child, Some(view));
        }
        canvas.pop_clip();
        state.recording.replay_at(canvas, offset);
    }

    /// Se o widget `id` aparece: ele e os ancestrais estão visíveis.
    pub fn shown(&self, id: NodeId) -> bool {
        self.tree.shown(id)
    }

    /// Widgets repintados desde a última chamada, sem repetição e na ordem da árvore: o id ou,
    /// sem id, o tipo (`"column"`).
    ///
    /// Conta quem foi gravado de novo (o visual ou o tamanho mudou) e quem o layout mudou de
    /// lugar e aparece na tela. Rolar um `ScrollViewer` não muda o layout: repinta só ele (as
    /// barras).
    pub fn repaints_since_last(&mut self) -> Vec<String> {
        let mut ids = std::mem::take(&mut self.repaints);
        ids.retain(|&id| self.tree.contains(id));
        ids.sort_unstable_by_key(|&id| self.tree.position(id));
        ids.dedup();
        ids.into_iter().map(|id| self.tree.name(id)).collect()
    }
}

impl State {
    fn new(tree: &Tree, id: NodeId) -> Self {
        Self {
            visible: tree.visible(id),
            opacity: 1.0,
            native: tree.widget(id).native(),
            ..Self::default()
        }
    }
}

/// Folga em volta de um widget ao decidir se ele aparece num `ScrollViewer`.
const CULL_MARGIN: f64 = 16.0;

/// Se os retângulos se encostam com área.
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

impl Drop for Ui {
    fn drop(&mut self) {
        self.scope.clear();
    }
}

impl std::fmt::Debug for Ui {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ui")
            .field("size", &self.size)
            .field("tree", &self.tree)
            .finish_non_exhaustive()
    }
}

/// Aplica a `color` a cor herdada de um ancestral. Desabilitado, as cores de texto do tema
/// também apagam (a descrição secundária do `SettingsCard`); uma cor fixa fica.
fn inherit(color: &mut Option<ThemeColor>, inherited: Option<ThemeColor>) {
    let disabled = ThemeColor::Token(Token::TextFillColorDisabled);
    let themed = matches!(
        color,
        Some(ThemeColor::Token(
            Token::TextFillColorPrimary
                | Token::TextFillColorSecondary
                | Token::TextFillColorTertiary
        ))
    );
    if color.is_none() || inherited == Some(disabled) && themed {
        *color = inherited.or(*color);
    }
}
