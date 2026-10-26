//! Layout flex via `taffy` (`gap`, `padding`, alinhamento e `grow`); re-mede só o que ficou dirty.
//!
//! O [`LayoutEngine`] espelha a [`Tree`] em árvores do taffy: uma para a janela e uma para o
//! conteúdo de cada `ScrollViewer` (e de cada contêiner com
//! [`layout_boundary`](tomni_ui_core::Widget::layout_boundary)), que na árvore de fora é uma
//! folha medida pelo conteúdo. As folhas guardam o tamanho medido pelo `Drawer`; mudar uma
//! medida marca só o caminho até a raiz, e o taffy reaproveita o cache do resto. Texto que
//! quebra linhas é medido de novo a cada largura oferecida (e a medida de cada largura fica
//! guardada até o texto mudar), e um widget oculto (`.visible(false)`) vira `display: none`.
//! A medida do conteúdo de cada árvore de dentro também fica guardada por tamanho pedido: um
//! layout pergunta o mesmo `ScrollViewer` em várias larguras, e um texto grande tem milhares
//! de nós.
//!
//! Fragmentos (`Show`, `For`) não têm nó: os filhos entram direto no contêiner de cima. Quando
//! a árvore troca os filhos de um fragmento, [`LayoutEngine::sync`] cria e remove só os nós
//! que mudaram.
//!
//! Cada camada (popup) tem uma árvore do taffy só dela e não ocupa espaço no pai. Aberta, ela
//! é medida com o espaço da janela e posta junto da origem (o widget de cima): embaixo ou em
//! cima, trocando de lado se não couber, ou no centro da janela.

use slotmap::SecondaryMap;
use taffy::prelude::{
    AvailableSpace, Dimension, LengthPercentage, LengthPercentageAuto, TaffyTree,
};
use taffy::{
    AlignItems, Display, FlexDirection, FlexWrap, JustifyContent, Style, compute_leaf_layout,
};
use tomni_ui_core::kurbo::{Point, Rect, Size};
use tomni_ui_core::{
    Align, Changes, Direction, Flex, Insets, Justify, NodeId, Placement, ScrollMode, Tree,
};

/// Espaço mínimo entre um popup e a borda da janela (ou da área segura).
const LAYER_MARGIN: f64 = 8.0;

type TaffyId = taffy::NodeId;

/// Como uma folha é medida.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Measure {
    /// Tamanho natural, medido pelo `Drawer`.
    Fixed(taffy::Size<f32>),
    /// A altura depende da largura: o tamanho vem do callback do [`LayoutEngine::compute`].
    Reflow,
    /// `ScrollViewer`: o tamanho natural é o do conteúdo, calculado na árvore dele.
    Scroll(ScrollMode),
    /// Contêiner com a árvore dele ([`Widget::layout_boundary`]): o tamanho é o do conteúdo,
    /// calculado como o de um contêiner comum, mas guardado por largura pedida.
    ///
    /// [`Widget::layout_boundary`]: tomni_ui_core::Widget::layout_boundary
    Boundary,
}

/// Medida de uma folha, guardada no nó do taffy.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Leaf {
    id: NodeId,
    measure: Measure,
}

/// Uma árvore do taffy: a da janela, a do conteúdo de um `ScrollViewer`, a de um contêiner
/// com a árvore dele ([`Measure::Boundary`]) ou a de uma camada.
#[derive(Debug)]
struct Pane {
    taffy: TaffyTree<Leaf>,
    /// Raiz: o widget raiz, na janela; um nó com o flex do `ScrollViewer`, no conteúdo dele; o
    /// nó do widget, numa camada.
    root: TaffyId,
    /// Estilo da raiz sem o tamanho, que muda a cada medida.
    style: Style,
    /// Medidas do conteúdo de um `ScrollViewer` já feitas, as usadas por último no fim. Num
    /// layout só, o taffy pede o `ScrollViewer` em várias larguras (a mínima, a de cada
    /// sondagem do flex de fora, a final), e cada pedido refaria o layout do conteúdo
    /// inteiro: a pré-visualização de um texto grande tem milhares de nós. Vale até o
    /// conteúdo mudar ([`LayoutEngine::touch`]).
    measured: Vec<(ScrollKey, taffy::Size<f32>)>,
    /// O último layout final do conteúdo (o modo, o tamanho e o tamanho do conteúdo), enquanto
    /// as posições guardadas no taffy ainda são as dele: sem mudança, o próximo
    /// `layout_content` no mesmo tamanho não precisa refazer nada.
    laid_out: Option<(Option<ScrollMode>, taffy::Size<f32>, Size)>,
    /// Quantas vezes a árvore foi calculada (nos testes, que contam o trabalho).
    #[cfg(test)]
    computed: usize,
}

/// O que decide a medida do conteúdo de um `ScrollViewer`: o tamanho e o mínimo da raiz e o
/// espaço disponível.
type ScrollKey = (
    taffy::Size<Dimension>,
    taffy::Size<LengthPercentageAuto>,
    taffy::Size<AvailableSpace>,
);

/// Quantas medidas do conteúdo de cada `ScrollViewer` ficam guardadas.
const SCROLL_MEASURES: usize = 32;

impl Pane {
    fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.enable_rounding();
        Self {
            taffy,
            root: TaffyId::from(u64::MAX),
            style: Style::default(),
            measured: Vec::new(),
            laid_out: None,
            #[cfg(test)]
            computed: 0,
        }
    }
}

/// Layout de uma [`Tree`].
#[derive(Debug)]
pub struct LayoutEngine {
    /// Árvore da janela.
    window: Pane,
    /// Árvore do conteúdo de cada `ScrollViewer` e de cada camada.
    panes: SecondaryMap<NodeId, Pane>,
    /// Camadas.
    layers: SecondaryMap<NodeId, ()>,
    /// Onde está o nó de cada widget: `None` na árvore da janela, `Some(sv)` na do
    /// `ScrollViewer` ou da camada `sv` (uma camada fica na própria árvore). Fragmentos não
    /// têm nó.
    nodes: SecondaryMap<NodeId, (Option<NodeId>, TaffyId)>,
    root: NodeId,
    /// Padding próprio da raiz, somado à safe area.
    root_padding: Insets,
    viewport: Size,
    safe_area: Insets,
    dirty: bool,
    /// Medidas das folhas que quebram linhas, por largura oferecida (`None` é numa linha só).
    /// O taffy pede a mesma folha várias vezes num layout (o mínimo, o natural, a largura
    /// final, e de novo em cada medida de um `ScrollViewer` de fora); medir é refazer o texto
    /// inteiro. Vale até [`LayoutEngine::set_reflow`] dizer que o conteúdo mudou.
    measures: SecondaryMap<NodeId, Vec<(Option<u64>, Size)>>,
}

/// Quantas larguras de cada folha ficam guardadas: as do layout atual e as de algumas larguras
/// de antes, enquanto a janela muda de tamanho. Um layout só pede várias: o mínimo, o natural,
/// a largura final e as sondagens de um flex em linha (o item de uma lista com o marcador pede
/// sete larguras diferentes). Com menos lugares que isso, cada layout expulsaria a largura que
/// o próximo vai pedir e nenhuma medida seria aproveitada; por isso a fila é das usadas por
/// último (a medida achada vai para o fim) e tem folga.
const MEASURES_PER_LEAF: usize = 16;

impl LayoutEngine {
    /// Espelha `tree`. As folhas começam com tamanho zero, até o primeiro
    /// [`LayoutEngine::set_measure`].
    pub fn new(tree: &Tree) -> Self {
        let mut engine = Self {
            window: Pane::new(),
            panes: SecondaryMap::new(),
            layers: SecondaryMap::new(),
            nodes: SecondaryMap::new(),
            root: tree.root(),
            root_padding: tree
                .widget(tree.root())
                .flex()
                .map_or(Insets::ZERO, |f| f.padding),
            viewport: Size::ZERO,
            safe_area: Insets::ZERO,
            dirty: true,
            measures: SecondaryMap::new(),
        };
        let ids: Vec<NodeId> = tree.ids().collect();
        for &id in &ids {
            engine.create(tree, id);
        }
        engine.window.root = engine.nodes[tree.root()].1;
        engine.window.style = engine
            .window
            .taffy
            .style(engine.window.root)
            .expect("a raiz existe")
            .clone();
        for &id in &ids {
            engine.relink(tree, id);
        }
        engine
    }

    /// Acompanha as mudanças de um [`Tree::reconcile`]: remove os nós de quem saiu, cria os de
    /// quem entrou e refaz só as listas de filhos que mudaram.
    pub fn sync(&mut self, tree: &Tree, changes: &Changes) {
        for &id in &changes.removed {
            // O pai sai antes dos filhos: a árvore de um `ScrollViewer` removido já foi junto.
            if let Some((pane, node)) = self.nodes.remove(id)
                && let Some(taffy) = self.taffy(pane)
            {
                taffy.remove(node).expect("o nó existe");
                self.touch(pane);
            }
            self.panes.remove(id);
            self.layers.remove(id);
            self.measures.remove(id);
        }
        for &id in &changes.added {
            self.create(tree, id);
        }
        let mut parents: Vec<NodeId> = changes
            .parents
            .iter()
            .chain(&changes.added)
            .filter_map(|&id| {
                if tree.widget(id).fragment() {
                    layout_parent(tree, id)
                } else {
                    Some(id)
                }
            })
            .collect();
        parents.sort_unstable_by_key(|&id| tree.position(id));
        parents.dedup();
        for id in parents {
            self.relink(tree, id);
        }
        self.dirty = true;
    }

    /// Cria o nó de `id`, sem os filhos.
    fn create(&mut self, tree: &Tree, id: NodeId) {
        let widget = tree.widget(id);
        if widget.fragment() {
            return;
        }
        let pane = pane_of(tree, id);
        let layer = tree.layer(id).is_some();
        if layer {
            self.panes.insert(id, Pane::new());
            self.layers.insert(id, ());
        }
        let mut style = Style::default();
        if !tree.visible(id) {
            style.display = Display::None;
        }
        let common = widget.common();
        style.flex_grow = common.grow as f32;
        style.size = taffy::Size {
            width: common.width.map_or(Dimension::auto(), length),
            height: common.height.map_or(Dimension::auto(), length),
        };
        if let Some(max) = common.max_width {
            style.max_size.width = LengthPercentageAuto::length(max as f32);
        }
        if let Some(parent) = layout_parent(tree, id)
            .filter(|_| !layer)
            .and_then(|p| tree.widget(p).flex())
        {
            // Com `grow` e sem tamanho fixo no eixo principal, a base é zero (o `flex: 1`
            // do CSS): os irmãos dividem o espaço na proporção do `grow`, em vez de dividir
            // só a sobra depois do conteúdo. O mínimo automático do flex continua valendo,
            // então o texto não é cortado.
            let fixed = match parent.direction {
                Direction::Row => common.width,
                Direction::Column => common.height,
            };
            if common.grow > 0.0 && fixed.is_none() {
                style.flex_basis = length(0.0);
            }
            // O `fill` estica quem não tem tamanho fixo: a `TitleBar` do `Form` fica com a
            // altura dela, e o conteúdo, com o resto.
            if parent.fill && fixed.is_none() {
                style.flex_grow = style.flex_grow.max(1.0);
            }
        }
        let scroller = widget.scroller().copied();
        let flex = widget.flex();
        let boundary = is_boundary(tree, id);
        let leaf = |measure| Leaf { id, measure };
        if layer {
            let pane = &mut self.panes[id];
            apply_flex(&mut style, &flex.unwrap_or_default());
            pane.root = pane
                .taffy
                .new_leaf(style.clone())
                .expect("o taffy não falha ao criar nós");
            pane.style = style;
            self.nodes.insert(id, (Some(id), pane.root));
            return;
        }
        let taffy = self.taffy(pane).expect("a árvore do pai existe");
        let node = match (scroller, flex) {
            (Some(scroller), _) => {
                // Folha medida pelo conteúdo, que encolhe até zero: numa `Column`, ocupa a
                // altura do conteúdo se couber e, se não, o que sobra.
                taffy.new_leaf_with_context(style, leaf(Measure::Scroll(scroller.mode())))
            }
            // O flex fica na raiz da árvore dele, como no `ScrollViewer`.
            (None, Some(_)) if boundary => {
                taffy.new_leaf_with_context(style, leaf(Measure::Boundary))
            }
            (None, Some(flex)) => {
                apply_flex(&mut style, &flex);
                taffy.new_leaf(style)
            }
            (None, None) => {
                // Folha: o tamanho natural vem do `Drawer`, e não encolhe abaixo dele.
                style.flex_shrink = 0.0;
                taffy.new_leaf_with_context(style, leaf(Measure::Fixed(taffy::Size::ZERO)))
            }
        }
        .expect("o taffy não falha ao criar nós");
        self.nodes.insert(id, (pane, node));
        if scroller.is_some() || boundary {
            let mut content = Pane::new();
            let mut style = Style::default();
            apply_flex(&mut style, &flex.unwrap_or_default());
            content.root = content
                .taffy
                .new_leaf(style.clone())
                .expect("o taffy não falha ao criar nós");
            content.style = style;
            self.panes.insert(id, content);
        }
    }

    /// Refaz a lista de filhos de `id` no taffy, com os fragmentos achatados.
    fn relink(&mut self, tree: &Tree, id: NodeId) {
        if tree.widget(id).fragment() {
            return;
        }
        let mut children = Vec::new();
        self.expand(tree, id, &mut children);
        let (pane, node) = match self.panes.get(id) {
            Some(content) => (Some(id), content.root),
            None => self.nodes[id],
        };
        if children.is_empty() && tree.children(id).is_empty() {
            return;
        }
        self.taffy(pane)
            .expect("a árvore existe")
            .set_children(node, &children)
            .expect("o nó existe");
        self.touch(pane);
    }

    /// Nós do taffy dos filhos de `id`, entrando nos fragmentos.
    fn expand(&self, tree: &Tree, id: NodeId, out: &mut Vec<TaffyId>) {
        for &child in tree.children(id) {
            if self.layers.contains_key(child) {
                // A camada fica na árvore dela.
                continue;
            }
            match self.nodes.get(child) {
                Some(&(_, node)) => out.push(node),
                None => self.expand(tree, child, out),
            }
        }
    }

    fn taffy(&mut self, pane: Option<NodeId>) -> Option<&mut TaffyTree<Leaf>> {
        match pane {
            None => Some(&mut self.window.taffy),
            Some(sv) => self.panes.get_mut(sv).map(|p| &mut p.taffy),
        }
    }

    /// Algo mudou na árvore `pane`: o `ScrollViewer` dono dela (e os de fora) precisa ser
    /// medido de novo.
    fn touch(&mut self, mut pane: Option<NodeId>) {
        self.dirty = true;
        while let Some(sv) = pane {
            if let Some(content) = self.panes.get_mut(sv) {
                content.measured.clear();
                content.laid_out = None;
            }
            if self.layers.contains_key(sv) {
                // A camada é medida de novo a cada `compute`.
                return;
            }
            let Some(&(outer, node)) = self.nodes.get(sv) else {
                return;
            };
            if let Some(taffy) = self.taffy(outer) {
                taffy.mark_dirty(node).expect("o nó existe");
            }
            pane = outer;
        }
    }

    fn node(&self, id: NodeId) -> (Option<NodeId>, TaffyId) {
        self.nodes[id]
    }

    /// Tamanho natural da folha `id`, medido pelo `Drawer`. Só marca o layout como sujo se
    /// mudou.
    pub fn set_measure(&mut self, id: NodeId, size: Size) {
        let size = taffy::Size {
            width: size.width as f32,
            height: size.height as f32,
        };
        self.set_leaf(id, Measure::Fixed(size));
    }

    /// Tamanho mínimo do contêiner `id`, que também tem tamanho natural
    /// ([`Widget::measured`](tomni_ui_core::Widget::measured)): o `Edit` com botões dentro.
    pub fn set_min_size(&mut self, id: NodeId, size: Size) {
        let (pane, node) = self.node(id);
        let taffy = self.taffy(pane).expect("a árvore existe");
        let mut style = taffy.style(node).expect("o nó existe").clone();
        let min = taffy::Size {
            width: LengthPercentageAuto::length(size.width as f32),
            height: LengthPercentageAuto::length(size.height as f32),
        };
        if style.min_size == min {
            return;
        }
        style.min_size = min;
        taffy.set_style(node, style).expect("o nó existe");
        self.touch(pane);
    }

    /// Largura de `id` que muda com o estado do widget
    /// ([`Widget::dynamic_width`](tomni_ui_core::Widget::dynamic_width)): com `Some`, fixa e
    /// sem encolher, mesmo que os filhos passem dela; com `None`, a do `.width()` ou automática.
    /// Só marca o layout como sujo se mudou.
    pub fn set_width(&mut self, id: NodeId, width: Option<f64>, fallback: Option<f64>) {
        let Some(&(pane, node)) = self.nodes.get(id) else {
            return;
        };
        let taffy = self.taffy(pane).expect("a árvore existe");
        let mut style = taffy.style(node).expect("o nó existe").clone();
        let (size, min, max, shrink) = match width {
            Some(width) => (
                length(width),
                LengthPercentageAuto::length(width as f32),
                LengthPercentageAuto::length(width as f32),
                0.0,
            ),
            None => (
                fallback.map_or(Dimension::auto(), length),
                LengthPercentageAuto::auto(),
                LengthPercentageAuto::auto(),
                1.0,
            ),
        };
        if style.size.width == size
            && style.min_size.width == min
            && style.max_size.width == max
            && style.flex_shrink == shrink
        {
            return;
        }
        style.size.width = size;
        style.min_size.width = min;
        style.max_size.width = max;
        style.flex_shrink = shrink;
        taffy.set_style(node, style).expect("o nó existe");
        self.touch(pane);
    }

    /// Base e mínimo de `id` no eixo do pai que mudam com o estado
    /// ([`Widget::dynamic_basis`](tomni_ui_core::Widget::dynamic_basis)): o widget começa
    /// com `basis` e encolhe até `min` se faltar espaço. Só marca o layout como sujo se mudou.
    pub fn set_basis(&mut self, tree: &Tree, id: NodeId, (basis, min): (f64, f64)) {
        let Some(&(pane, node)) = self.nodes.get(id) else {
            return;
        };
        let direction = layout_parent(tree, id)
            .and_then(|p| tree.widget(p).flex())
            .map_or(Direction::Column, |flex| flex.direction);
        let taffy = self.taffy(pane).expect("a árvore existe");
        let mut style = taffy.style(node).expect("o nó existe").clone();
        let before = style.clone();
        style.flex_basis = length(basis);
        style.flex_shrink = 1.0;
        let min = LengthPercentageAuto::length(min as f32);
        match direction {
            Direction::Row => style.min_size.width = min,
            Direction::Column => style.min_size.height = min,
        }
        if style == before {
            return;
        }
        taffy.set_style(node, style).expect("o nó existe");
        self.touch(pane);
    }

    /// Fator de crescimento de `id` que muda com o estado
    /// ([`Widget::dynamic_grow`](tomni_ui_core::Widget::dynamic_grow)); `None` volta ao
    /// `.grow()` do widget. Só marca o layout como sujo se mudou.
    pub fn set_grow(&mut self, id: NodeId, grow: Option<f64>, fallback: f64) {
        let Some(&(pane, node)) = self.nodes.get(id) else {
            return;
        };
        let taffy = self.taffy(pane).expect("a árvore existe");
        let mut style = taffy.style(node).expect("o nó existe").clone();
        let grow = grow.unwrap_or(fallback) as f32;
        if style.flex_grow == grow {
            return;
        }
        style.flex_grow = grow;
        taffy.set_style(node, style).expect("o nó existe");
        self.touch(pane);
    }

    /// A folha `id` quebra linhas e mudou de conteúdo: no próximo [`LayoutEngine::compute`],
    /// o callback mede de novo nas larguras que o layout oferecer. Ela também pode encolher
    /// abaixo da largura natural, até a palavra mais longa.
    pub fn set_reflow(&mut self, id: NodeId) {
        self.measures.remove(id);
        let (pane, node) = self.node(id);
        let taffy = self.taffy(pane).expect("a árvore existe");
        let leaf = Leaf {
            id,
            measure: Measure::Reflow,
        };
        if taffy.get_node_context(node) == Some(&leaf) {
            taffy.mark_dirty(node).expect("o nó existe");
            self.touch(pane);
            return;
        }
        let mut style = taffy.style(node).expect("o nó existe").clone();
        style.flex_shrink = 1.0;
        taffy.set_style(node, style).expect("o nó existe");
        self.set_leaf(id, Measure::Reflow);
    }

    fn set_leaf(&mut self, id: NodeId, measure: Measure) {
        let (pane, node) = self.node(id);
        let taffy = self.taffy(pane).expect("a árvore existe");
        let leaf = Leaf { id, measure };
        if taffy.get_node_context(node) == Some(&leaf) {
            return;
        }
        taffy
            .set_node_context(node, Some(leaf))
            .expect("o nó existe");
        self.touch(pane);
    }

    /// Mostra ou esconde `id`. Oculto, ele e os descendentes não ocupam espaço e ficam com
    /// retângulo vazio.
    pub fn set_visible(&mut self, id: NodeId, visible: bool) {
        let Some(&(pane, node)) = self.nodes.get(id) else {
            return;
        };
        // A raiz (o `Form`) nunca é uma grade.
        if id == self.root {
            self.window.style.display = if visible {
                Display::Flex
            } else {
                Display::None
            };
        }
        let taffy = self.taffy(pane).expect("a árvore existe");
        let mut style = taffy.style(node).expect("o nó existe").clone();
        let display = match (visible, style.grid_template_columns.is_empty()) {
            (false, _) => Display::None,
            (true, true) => Display::Flex,
            (true, false) => Display::Grid,
        };
        if style.display == display {
            return;
        }
        style.display = display;
        taffy.set_style(node, style).expect("o nó existe");
        self.touch(pane);
    }

    /// Tamanho da janela. A raiz ocupa a janela inteira.
    pub fn set_viewport(&mut self, size: Size) {
        if size != self.viewport {
            self.viewport = size;
            self.dirty = true;
        }
    }

    /// Área ocupada pelo sistema (barra de status, entalhe). Vira padding da raiz.
    pub fn set_safe_area(&mut self, insets: Insets) {
        if insets != self.safe_area {
            self.safe_area = insets;
            self.dirty = true;
        }
    }

    /// Se alguma medida, a estrutura, a janela ou a safe area mudou desde o último
    /// [`LayoutEngine::compute`].
    pub fn needs_layout(&self) -> bool {
        self.dirty
    }

    /// Calcula o layout, grava os retângulos na árvore e o tamanho do conteúdo de cada
    /// `ScrollViewer` ([`Tree::set_scroll_extent`]). Devolve, na ordem da árvore, os widgets
    /// cujo retângulo mudou.
    ///
    /// Os retângulos são os do layout, sem a rolagem ([`Tree::layout_rect`]): rolar não muda o
    /// layout.
    ///
    /// `measure(id, largura)` mede as folhas marcadas com [`LayoutEngine::set_reflow`]: o
    /// tamanho do texto quebrado na largura dada, ou numa linha só com `None`.
    pub fn compute(
        &mut self,
        tree: &mut Tree,
        mut measure: impl FnMut(NodeId, Option<f64>) -> Size,
    ) -> Vec<NodeId> {
        let mut measures = std::mem::take(&mut self.measures);
        let mut measure = |id: NodeId, width: Option<f64>| {
            let key = width.map(f64::to_bits);
            let mut entry = measures.entry(id).map(|e| e.or_default());
            if let Some(entry) = &mut entry
                && let Some(i) = entry.iter().position(|(k, _)| *k == key)
            {
                let hit = entry.remove(i);
                entry.push(hit);
                return hit.1;
            }
            let size = measure(id, width);
            if let Some(entry) = entry {
                if entry.len() == MEASURES_PER_LEAF {
                    entry.remove(0);
                }
                entry.push((key, size));
            }
            size
        };
        let mut style = self.window.style.clone();
        style.size = taffy::Size {
            width: length(self.viewport.width),
            height: length(self.viewport.height),
        };
        style.padding = to_rect(self.root_padding + self.safe_area);
        self.window
            .taffy
            .set_style(self.window.root, style)
            .expect("a raiz existe");
        let available = taffy::Size {
            width: AvailableSpace::Definite(self.viewport.width as f32),
            height: AvailableSpace::Definite(self.viewport.height as f32),
        };
        compute_pane(&mut self.window, &mut self.panes, available, &mut measure);
        self.dirty = false;

        let ids: Vec<NodeId> = tree.ids().collect();
        let mut changed = Vec::new();
        let mut origins: SecondaryMap<NodeId, Point> = SecondaryMap::new();
        // Widgets de camadas fechadas: ficam com retângulo vazio.
        let mut closed: SecondaryMap<NodeId, ()> = SecondaryMap::new();
        for &id in &ids {
            let Some(&(pane, node)) = self.nodes.get(id) else {
                continue;
            };
            let parent = layout_parent(tree, id);
            if parent.is_some_and(|p| closed.contains_key(p))
                || self.layers.contains_key(id) && !tree.shown(id)
            {
                closed.insert(id, ());
                origins.insert(id, Point::ZERO);
                if tree.set_rect(id, Rect::ZERO) {
                    changed.push(id);
                }
                continue;
            }
            if self.layers.contains_key(id) {
                let Some(rect) = self.place_layer(tree, id, &mut measure) else {
                    // Uma sobreposição que não cabe na origem não aparece.
                    closed.insert(id, ());
                    origins.insert(id, Point::ZERO);
                    if tree.set_rect(id, Rect::ZERO) {
                        changed.push(id);
                    }
                    continue;
                };
                origins.insert(id, rect.origin());
                if tree.set_rect(id, rect) {
                    changed.push(id);
                }
                continue;
            }
            let pane = match pane {
                None => &self.window,
                Some(sv) => &self.panes[sv],
            };
            let layout = pane.taffy.layout(node).expect("o nó existe");
            let base = parent.map_or(Point::ZERO, |p| origins[p]);
            let origin = base + (f64::from(layout.location.x), f64::from(layout.location.y));
            origins.insert(id, origin);
            let size = Size::new(f64::from(layout.size.width), f64::from(layout.size.height));
            if let Some(mut content) = self.panes.remove(id) {
                let mode = tree.scroller(id).map(|s| s.mode());
                let extent =
                    layout_content(&mut content, &mut self.panes, mode, size, &mut measure);
                self.panes.insert(id, content);
                if mode.is_some() {
                    tree.set_scroll_extent(id, extent, size);
                }
            }
            if tree.set_rect(id, Rect::from_origin_size(origin, size)) {
                changed.push(id);
            }
        }
        // Fragmentos: a união dos filhos, de dentro para fora.
        for &id in ids.iter().rev() {
            if self.nodes.contains_key(id) {
                continue;
            }
            let rect = tree
                .children(id)
                .iter()
                .filter(|&&child| !self.layers.contains_key(child))
                .map(|&child| tree.layout_rect(child))
                .reduce(|a, b| a.union(b))
                .unwrap_or_else(|| {
                    let origin = layout_parent(tree, id).map_or(Point::ZERO, |p| origins[p]);
                    Rect::from_origin_size(origin, Size::ZERO)
                });
            if tree.set_rect(id, rect) {
                changed.push(id);
            }
        }
        self.measures = measures;
        changed.sort_unstable_by_key(|&id| tree.position(id));
        changed
    }

    /// Mede a camada aberta `id` e decide onde ela fica, na janela. `None` é uma sobreposição
    /// ([`Placement::Over`]) que não cabe dentro da origem: ela não aparece.
    fn place_layer(
        &mut self,
        tree: &Tree,
        id: NodeId,
        measure: &mut MeasureFn<'_>,
    ) -> Option<Rect> {
        let layer = tree.layer(id).expect("é uma camada");
        let area = Rect::from_origin_size(Point::ZERO, self.viewport);
        let area = Rect::new(
            area.x0 + self.safe_area.left + LAYER_MARGIN,
            area.y0 + self.safe_area.top + LAYER_MARGIN,
            area.x1 - self.safe_area.right - LAYER_MARGIN,
            area.y1 - self.safe_area.bottom - LAYER_MARGIN,
        );
        // O menu de contexto aberto pelo botão direito fica com o canto no ponteiro.
        let point = tree.anchor_rect(id);
        let anchor = match point {
            Some(rect) => rect,
            None => layout_parent(tree, id).map_or(area, |p| tree.rect(p)),
        };
        let gap = if point.is_some() { 0.0 } else { layer.gap };
        let min_width = if layer.match_width {
            layer.min_size.width.max(anchor.width())
        } else {
            layer.min_size.width
        };
        let max_width = layer.max_size.width.min(area.width()).max(0.0);
        let mut pane = self.panes.remove(id).expect("a camada tem árvore");
        // Com `width`, a largura fica fixa: o texto que quebra linhas é medido nela.
        let mut size_with = |pane: &mut Pane, width: Option<f64>, max_height: f64| {
            let mut style = pane.style.clone();
            style.display = Display::Flex;
            let (min_width, max_width) = width.map_or((min_width, max_width), |w| (w, w));
            style.min_size = taffy::Size {
                width: LengthPercentageAuto::length(min_width.min(max_width) as f32),
                height: LengthPercentageAuto::length(layer.min_size.height.min(max_height) as f32),
            };
            style.max_size = taffy::Size {
                width: LengthPercentageAuto::length(max_width as f32),
                height: LengthPercentageAuto::length(
                    layer.max_size.height.min(max_height).max(0.0) as f32,
                ),
            };
            pane.taffy
                .set_style(pane.root, style)
                .expect("a raiz existe");
            let available = taffy::Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            };
            compute_pane(pane, &mut self.panes, available, measure);
            let size = pane.taffy.layout(pane.root).expect("a raiz existe").size;
            Size::new(f64::from(size.width), f64::from(size.height))
        };
        // A largura sai da primeira medida (a natural, limitada); a altura, da segunda, com o
        // texto já quebrado nessa largura.
        let width = size_with(&mut pane, None, area.height()).width;
        let mut size = size_with(&mut pane, Some(width), area.height());
        let below = area.y1 - (anchor.y1 + gap);
        let above = anchor.y0 - gap - area.y0;
        if layer.placement == Placement::Over
            && (size.width + 2.0 * gap > anchor.width() + 0.5
                || size.height + 2.0 * gap > anchor.height() + 0.5)
        {
            self.panes.insert(id, pane);
            return None;
        }
        let (x, y) = match layer.placement {
            Placement::Over => (
                anchor.center().x - size.width / 2.0,
                anchor.center().y - size.height / 2.0,
            ),
            Placement::Center => (
                area.center().x - size.width / 2.0,
                area.center().y - size.height / 2.0,
            ),
            Placement::End => (anchor.x1 + layer.offset.x, anchor.y0 + layer.offset.y),
            Placement::Side => {
                let right = anchor.x1 + gap;
                let x = if right + size.width <= area.x1 {
                    right
                } else {
                    anchor.x0 - gap - size.width
                };
                (x, anchor.y0 + layer.offset.y)
            }
            placement => {
                let prefer_below = placement == Placement::Below;
                let (first, second) = if prefer_below {
                    (below, above)
                } else {
                    (above, below)
                };
                let goes_below = if size.height <= first {
                    prefer_below
                } else if size.height <= second {
                    !prefer_below
                } else if point.is_some() {
                    // Menu de contexto no ponteiro sem caber em nenhum lado: fica inteiro,
                    // empurrado para dentro da janela pelo ajuste do fim, como o `MenuFlyout`
                    // do WinUI. Só rola se for maior que a janela (a medida já para nela).
                    true
                } else if below.max(above) >= size.height / 2.0 {
                    // Não cabe em nenhum lado: fica no que tem mais espaço, e rola.
                    let goes_below = below >= above;
                    size = size_with(
                        &mut pane,
                        Some(width),
                        if goes_below { below } else { above },
                    );
                    goes_below
                } else {
                    // Nenhum lado tem nem metade: fica do tamanho natural e cobre a origem,
                    // empurrado para dentro da janela logo abaixo.
                    prefer_below
                };
                let y = if goes_below {
                    anchor.y1 + gap
                } else {
                    anchor.y0 - gap - size.height
                };
                let x = if point.is_some() && anchor.x0 + size.width > area.x1 {
                    // No ponteiro, sem espaço à direita: vira para a esquerda dele.
                    anchor.x0 - size.width
                } else if prefer_below {
                    anchor.x0
                } else {
                    anchor.center().x - size.width / 2.0
                };
                (x, y)
            }
        };
        self.panes.insert(id, pane);
        let x = x.min(area.x1 - size.width).max(area.x0).round();
        let y = y.min(area.y1 - size.height).max(area.y0).round();
        Some(Rect::from_origin_size((x, y), size))
    }
}

/// Ancestral mais próximo que não é fragmento: o contêiner onde `id` entra no layout.
fn layout_parent(tree: &Tree, id: NodeId) -> Option<NodeId> {
    let mut node = tree.parent(id);
    while let Some(parent) = node {
        if !tree.widget(parent).fragment() {
            return Some(parent);
        }
        node = tree.parent(parent);
    }
    None
}

/// `ScrollViewer` ou camada mais próxima em volta de `id` (a própria `id`, numa camada): a
/// árvore do taffy onde fica o nó dele.
fn pane_of(tree: &Tree, id: NodeId) -> Option<NodeId> {
    if tree.layer(id).is_some() {
        return Some(id);
    }
    let mut node = tree.parent(id);
    while let Some(parent) = node {
        if tree.scroller(parent).is_some()
            || tree.layer(parent).is_some()
            || is_boundary(tree, parent)
        {
            return Some(parent);
        }
        node = tree.parent(parent);
    }
    None
}

/// Se `id` é um contêiner com a árvore dele ([`Measure::Boundary`]).
fn is_boundary(tree: &Tree, id: NodeId) -> bool {
    let widget = tree.widget(id);
    widget.layout_boundary()
        && widget.flex().is_some()
        && widget.scroller().is_none()
        && tree.layer(id).is_none()
        && !widget.fragment()
}

type MeasureFn<'a> = dyn FnMut(NodeId, Option<f64>) -> Size + 'a;

/// Calcula a árvore `pane`. Os `ScrollViewer` dentro dela são medidos pelas árvores deles, que
/// saem de `panes` enquanto são calculadas.
fn compute_pane(
    pane: &mut Pane,
    panes: &mut SecondaryMap<NodeId, Pane>,
    available: taffy::Size<AvailableSpace>,
    measure: &mut MeasureFn<'_>,
) {
    #[cfg(test)]
    {
        pane.computed += 1;
    }
    pane.taffy
        .compute_layout_with_measure(pane.root, available, |inputs, _, leaf, style| {
            let Some(&mut leaf) = leaf else {
                return compute_leaf_layout(inputs, style, |_, _| 0.0, |_, _| taffy::Size::ZERO);
            };
            compute_leaf_layout(
                inputs,
                style,
                |_, _| 0.0,
                |known, available| {
                    let size = match leaf.measure {
                        Measure::Fixed(size) => size,
                        Measure::Reflow => {
                            // A palavra mais longa é o mínimo: quebrar em largura 0 dá isso.
                            let width = known.width.or(match available.width {
                                AvailableSpace::Definite(w) => Some(w),
                                AvailableSpace::MinContent => Some(0.0),
                                AvailableSpace::MaxContent => None,
                            });
                            let size = measure(leaf.id, width.map(f64::from));
                            taffy::Size {
                                width: size.width as f32,
                                height: size.height as f32,
                            }
                        }
                        Measure::Scroll(mode) => {
                            measure_scroll(panes, leaf.id, Some(mode), known, available, measure)
                        }
                        Measure::Boundary => {
                            measure_scroll(panes, leaf.id, None, known, available, measure)
                        }
                    };
                    taffy::Size {
                        width: known.width.unwrap_or(size.width),
                        height: known.height.unwrap_or(size.height),
                    }
                },
            )
        })
        .expect("o layout não falha");
}

/// Tamanho natural do `ScrollViewer` `id`: o do conteúdo, mas zero no eixo que rola quando o
/// layout pergunta o mínimo. Assim ele encolhe até caber, em vez de empurrar a janela.
///
/// Sem `mode`, `id` é um contêiner com a árvore dele ([`Measure::Boundary`]): o conteúdo é
/// medido como o de um contêiner comum, com o tamanho conhecido e o espaço que o layout der.
fn measure_scroll(
    panes: &mut SecondaryMap<NodeId, Pane>,
    id: NodeId,
    mode: Option<ScrollMode>,
    known: taffy::Size<Option<f32>>,
    available: taffy::Size<AvailableSpace>,
    measure: &mut MeasureFn<'_>,
) -> taffy::Size<f32> {
    let Some(mut content) = panes.remove(id) else {
        return taffy::Size::ZERO;
    };
    let (scrolls_x, scrolls_y) = (
        mode.is_some_and(ScrollMode::x),
        mode.is_some_and(ScrollMode::y),
    );
    let axis = |scrolls: bool, known: Option<f32>, available: AvailableSpace| {
        if scrolls {
            // O conteúdo tem o tamanho que precisar, e pelo menos o da área visível.
            (
                Dimension::auto(),
                known.map_or(LengthPercentageAuto::auto(), LengthPercentageAuto::length),
                AvailableSpace::MaxContent,
            )
        } else if mode.is_none() {
            (
                known.map_or(Dimension::auto(), Dimension::length),
                LengthPercentageAuto::auto(),
                known.map_or(available, AvailableSpace::Definite),
            )
        } else {
            let size = known.or(match available {
                AvailableSpace::Definite(v) => Some(v),
                _ => None,
            });
            (
                size.map_or(Dimension::auto(), Dimension::length),
                LengthPercentageAuto::auto(),
                size.map_or(available, AvailableSpace::Definite),
            )
        }
    };
    let (width, min_width, avail_width) = axis(scrolls_x, known.width, available.width);
    let (height, min_height, avail_height) = axis(scrolls_y, known.height, available.height);
    // Na rolagem vertical, a altura conhecida só vira o mínimo do conteúdo, e a altura medida
    // não é usada quando ela é conhecida (`compute_pane` fica com a conhecida): o mínimo não
    // muda a medida, e tirá-lo da chave faz as sondagens de alturas diferentes (o flex de fora
    // pergunta o mínimo em cada altura que testa) caírem na mesma medida.
    let min_height = if scrolls_y {
        LengthPercentageAuto::auto()
    } else {
        min_height
    };
    let key: ScrollKey = (
        taffy::Size { width, height },
        taffy::Size {
            width: min_width,
            height: min_height,
        },
        taffy::Size {
            width: avail_width,
            height: avail_height,
        },
    );
    let size = if let Some(i) = content.measured.iter().position(|(k, _)| *k == key) {
        let hit = content.measured.remove(i);
        content.measured.push(hit);
        hit.1
    } else {
        let mut style = content.style.clone();
        style.size = key.0;
        style.min_size = key.1;
        content
            .taffy
            .set_style(content.root, style)
            .expect("a raiz existe");
        compute_pane(&mut content, panes, key.2, measure);
        // As posições guardadas agora são as desta medida, não as do layout final.
        content.laid_out = None;
        let size = content
            .taffy
            .layout(content.root)
            .expect("a raiz existe")
            .size;
        if content.measured.len() == SCROLL_MEASURES {
            content.measured.remove(0);
        }
        content.measured.push((key, size));
        size
    };
    panes.insert(id, content);
    let min = |scrolls: bool, available: AvailableSpace, size: f32| {
        if scrolls && available == AvailableSpace::MinContent {
            0.0
        } else {
            size
        }
    };
    taffy::Size {
        width: min(scrolls_x, available.width, size.width),
        height: min(scrolls_y, available.height, size.height),
    }
}

/// Layout final do conteúdo de um `ScrollViewer` (ou, sem `mode`, de um contêiner com a árvore
/// dele) de tamanho `viewport`. Devolve o tamanho do conteúdo.
fn layout_content(
    content: &mut Pane,
    panes: &mut SecondaryMap<NodeId, Pane>,
    mode: Option<ScrollMode>,
    viewport: Size,
    measure: &mut MeasureFn<'_>,
) -> Size {
    let viewport = taffy::Size {
        width: viewport.width as f32,
        height: viewport.height as f32,
    };
    if let Some((last_mode, last_viewport, extent)) = content.laid_out
        && last_mode == mode
        && last_viewport == viewport
    {
        // Nada mudou no conteúdo nem no tamanho desde o último layout final, e nenhuma medida
        // mexeu nas posições: elas continuam valendo. Os `ScrollViewer` de dentro ainda passam
        // pelo `layout_content` deles, no `compute`.
        return extent;
    }
    let (scrolls_x, scrolls_y) = (
        mode.is_some_and(ScrollMode::x),
        mode.is_some_and(ScrollMode::y),
    );
    let available = taffy::Size {
        width: AvailableSpace::Definite(viewport.width),
        height: AvailableSpace::Definite(viewport.height),
    };
    // A mesma conta da medida, com o tamanho já decidido.
    let mut style = content.style.clone();
    let axis = |scrolls: bool, size: f32| {
        if scrolls {
            (Dimension::auto(), LengthPercentageAuto::length(size))
        } else {
            (Dimension::length(size), LengthPercentageAuto::auto())
        }
    };
    let (width, min_width) = axis(scrolls_x, viewport.width);
    let (height, min_height) = axis(scrolls_y, viewport.height);
    style.size = taffy::Size { width, height };
    style.min_size = taffy::Size {
        width: min_width,
        height: min_height,
    };
    content
        .taffy
        .set_style(content.root, style)
        .expect("a raiz existe");
    let avail = taffy::Size {
        width: if scrolls_x {
            AvailableSpace::MaxContent
        } else {
            available.width
        },
        height: if scrolls_y {
            AvailableSpace::MaxContent
        } else {
            available.height
        },
    };
    compute_pane(content, panes, avail, measure);
    let size = content
        .taffy
        .layout(content.root)
        .expect("a raiz existe")
        .size;
    let extent = Size::new(f64::from(size.width), f64::from(size.height));
    content.laid_out = Some((mode, viewport, extent));
    extent
}

fn apply_flex(style: &mut Style, flex: &Flex) {
    if let Some(min) = flex.min_column_width {
        // `repeat(auto-fill, minmax(min, 1fr))`: tantas colunas quantas couberem, dividindo a
        // sobra. As linhas são `1fr`: sem altura definida, todas ficam da altura da mais alta.
        use taffy::style_helpers::{fr, length as track, minmax, repeat};
        if style.display != Display::None {
            style.display = Display::Grid;
        }
        style.grid_template_columns = vec![repeat(
            taffy::RepetitionCount::AutoFill,
            vec![minmax(track(min as f32), fr(1.0))],
        )];
        style.grid_auto_rows = vec![minmax(taffy::MinTrackSizingFunction::auto(), fr(1.0))];
    }
    style.flex_wrap = if flex.wrap {
        FlexWrap::Wrap
    } else {
        FlexWrap::NoWrap
    };
    style.flex_direction = match flex.direction {
        Direction::Column => FlexDirection::Column,
        Direction::Row => FlexDirection::Row,
    };
    let gap = LengthPercentage::length(flex.gap as f32);
    style.gap = taffy::Size {
        width: gap,
        height: gap,
    };
    style.padding = to_rect(flex.padding);
    style.align_items = match flex.align {
        Align::Start => AlignItems::FLEX_START,
        Align::Center => AlignItems::CENTER,
        Align::End => AlignItems::FLEX_END,
        Align::Stretch => AlignItems::STRETCH,
    };
    style.justify_content = match flex.justify {
        Justify::Start => JustifyContent::FLEX_START,
        Justify::Center => JustifyContent::CENTER,
        Justify::End => JustifyContent::FLEX_END,
        Justify::SpaceBetween => JustifyContent::SPACE_BETWEEN,
    };
}

fn length(value: f64) -> Dimension {
    Dimension::length(value as f32)
}

fn to_rect(insets: Insets) -> taffy::Rect<LengthPercentage> {
    let side = |v: f64| LengthPercentage::length(v as f32);
    taffy::Rect {
        left: side(insets.left),
        right: side(insets.right),
        top: side(insets.top),
        bottom: side(insets.bottom),
    }
}

#[cfg(test)]
mod tests {
    use tomni_ui_core::{Common, Look, Row, ScrollViewer, View, Widget};
    use tomni_ui_reactive::Scope;

    use super::*;

    /// Contêiner em coluna, com ou sem a árvore dele.
    struct Bloco {
        common: Common,
        boundary: bool,
        children: Vec<Box<dyn Widget>>,
    }

    impl Bloco {
        fn new(boundary: bool) -> Self {
            Self {
                common: Common::default(),
                boundary,
                children: Vec::new(),
            }
        }

        fn child(mut self, child: impl View) -> Self {
            self.children.push(Box::new(child));
            self
        }
    }

    impl Widget for Bloco {
        fn kind(&self) -> &'static str {
            "bloco"
        }

        fn common(&self) -> &Common {
            &self.common
        }

        fn look(&self) -> Look {
            Look::Column
        }

        fn flex(&self) -> Option<Flex> {
            Some(Flex {
                direction: Direction::Column,
                gap: 4.0,
                padding: Insets::all(2.0),
                ..Flex::default()
            })
        }

        fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
            std::mem::take(&mut self.children)
        }

        fn layout_boundary(&self) -> bool {
            self.boundary
        }
    }

    /// Folha: a de tipo `texto` quebra linhas; a `lateral` tem tamanho fixo.
    struct Folha {
        common: Common,
        kind: &'static str,
    }

    impl Folha {
        fn texto() -> Self {
            Self {
                common: Common::default(),
                kind: "texto",
            }
        }
    }

    impl Widget for Folha {
        fn kind(&self) -> &'static str {
            self.kind
        }

        fn common(&self) -> &Common {
            &self.common
        }

        fn look(&self) -> Look {
            Look::Column
        }
    }

    /// Um texto de 300 px numa linha, com palavras de até 50 px e linhas de 20 px.
    fn texto(width: Option<f64>) -> Size {
        let natural = 300.0;
        let width = width.unwrap_or(natural).max(50.0);
        let lines = (natural / width).ceil().max(1.0);
        Size::new(width.min(natural), 20.0 * lines)
    }

    /// Uma barra lateral e, ao lado, um `ScrollViewer` com `blocos` blocos de dois textos: a
    /// linha pergunta o `ScrollViewer` em várias larguras, como a página Notas.
    struct Cena {
        tree: Tree,
        engine: LayoutEngine,
        _scope: Scope,
    }

    impl Cena {
        fn new(boundary: bool, blocos: usize) -> Self {
            let scope = Scope::root();
            let root = scope.run(|| {
                let mut conteudo = Bloco::new(false);
                for _ in 0..blocos {
                    conteudo = conteudo.child(
                        Bloco::new(boundary)
                            .child(Folha::texto())
                            .child(Folha::texto()),
                    );
                }
                let lateral = Folha {
                    common: Common::default(),
                    kind: "lateral",
                };
                Box::new(
                    Row::new()
                        .child(lateral)
                        .child(ScrollViewer::new().grow(1).child(conteudo)),
                ) as Box<dyn Widget>
            });
            let tree = Tree::mount(root, &scope);
            let mut engine = LayoutEngine::new(&tree);
            engine.set_viewport(Size::new(800.0, 600.0));
            let ids: Vec<NodeId> = tree.ids().collect();
            for id in ids {
                match tree.widget(id).kind() {
                    "texto" => engine.set_reflow(id),
                    "lateral" => engine.set_measure(id, Size::new(100.0, 20.0)),
                    _ => {}
                }
            }
            Self {
                tree,
                engine,
                _scope: scope,
            }
        }

        /// Faz o layout e devolve as folhas medidas, uma por medida.
        fn compute(&mut self) -> Vec<NodeId> {
            let mut measured = Vec::new();
            self.engine.compute(&mut self.tree, |id, width| {
                measured.push(id);
                texto(width)
            });
            measured
        }

        fn of_kind(&self, kind: &str) -> Vec<NodeId> {
            self.tree
                .ids()
                .filter(|&id| self.tree.widget(id).kind() == kind)
                .collect()
        }

        /// Quantas vezes cada árvore de dentro já foi calculada, na ordem da árvore.
        fn computed(&self) -> Vec<(NodeId, usize)> {
            self.tree
                .ids()
                .filter_map(|id| self.engine.panes.get(id).map(|p| (id, p.computed)))
                .collect()
        }

        fn rects(&self) -> Vec<(&'static str, Rect)> {
            self.tree
                .ids()
                .map(|id| (self.tree.widget(id).kind(), self.tree.layout_rect(id)))
                .collect()
        }
    }

    #[test]
    fn layout_nas_mesmas_larguras_nao_mede_de_novo() {
        for boundary in [false, true] {
            let mut cena = Cena::new(boundary, 30);
            assert!(!cena.compute().is_empty());
            let antes = cena.computed();
            // Só a altura da janela muda: as larguras são as mesmas.
            cena.engine.set_viewport(Size::new(800.0, 500.0));
            assert_eq!(cena.compute(), [], "nenhum texto é medido de novo");
            let depois = cena.computed();
            let scroll = cena.of_kind("scroll_viewer")[0];
            for ((id, a), (_, d)) in antes.iter().zip(&depois) {
                if *id == scroll {
                    // O layout final da área visível nova.
                    assert_eq!(*d, a + 1);
                } else {
                    assert_eq!(d, a, "o bloco não é calculado de novo");
                }
            }
        }
    }

    #[test]
    fn mudar_um_texto_mede_so_ele_e_calcula_so_o_bloco_dele() {
        let mut cena = Cena::new(true, 30);
        cena.compute();
        let antes = cena.computed();
        let textos = cena.of_kind("texto");
        let mudou = textos[11];
        let bloco = cena.tree.parent(mudou).expect("o texto está num bloco");
        cena.engine.set_reflow(mudou);
        let medidas = cena.compute();
        assert!(!medidas.is_empty());
        assert!(
            medidas.iter().all(|&id| id == mudou),
            "só o texto que mudou é medido: {medidas:?}"
        );
        let blocos = cena.of_kind("bloco");
        for ((id, a), (_, d)) in antes.iter().zip(cena.computed()) {
            if blocos.contains(id) && *id != bloco {
                assert_eq!(d, *a, "os outros blocos não são calculados de novo");
            } else {
                assert!(d > *a);
            }
        }
    }

    #[test]
    fn conteiner_com_a_arvore_dele_fica_no_mesmo_lugar_e_tamanho() {
        let mut comum = Cena::new(false, 12);
        let mut separado = Cena::new(true, 12);
        for (largura, altura) in [
            (800.0, 600.0),
            (330.0, 600.0),
            (180.0, 90.0),
            (1200.0, 300.0),
        ] {
            for cena in [&mut comum, &mut separado] {
                cena.engine.set_viewport(Size::new(largura, altura));
                cena.compute();
            }
            assert_eq!(comum.rects(), separado.rects(), "em {largura}x{altura}");
        }
        // E depois de um texto mudar de tamanho.
        for cena in [&mut comum, &mut separado] {
            let texto = cena.of_kind("texto")[5];
            cena.engine.set_reflow(texto);
            cena.engine.set_viewport(Size::new(420.0, 600.0));
            cena.compute();
        }
        assert_eq!(comum.rects(), separado.rects());
    }
}
