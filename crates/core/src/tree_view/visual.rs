//! O que o `Drawer` e as ferramentas precisam saber do `TreeView` e das linhas dele, e a
//! geometria que o widget (o clique no chevron) e o tema (o desenho) dividem.

use std::sync::Arc;

use kurbo::Rect;

use super::rows::TreeFlat;
use crate::button::ButtonState;
use crate::density::Density;

/// Altura de uma linha: o fundo de 40 px com a margem de 4,2 do WinUI. Como o
/// `NavigationView` (44), passa no alvo de toque do lint.
pub const TREE_VIEW_ITEM_HEIGHT: f64 = 44.0;
/// Altura de uma linha no modo compacto: o `TreeViewItemMinHeight` do WinUI compacto, sem
/// margem.
pub const TREE_VIEW_ITEM_HEIGHT_COMPACT: f64 = 24.0;
/// Recuo de cada nível (`UpdateIndentation` do WinUI).
pub const TREE_VIEW_INDENT: f64 = 16.0;
/// Coluna do chevron: o padding de 14 dos lados mais o glifo de 12.
pub const TREE_VIEW_CHEVRON_COLUMN: f64 = 40.0;
/// Lado do ícone da linha.
pub const TREE_VIEW_ICON: f64 = 16.0;
/// Espaço entre o ícone e o rótulo.
pub const TREE_VIEW_ICON_GAP: f64 = 8.0;
/// Margem do fundo da linha, dos lados e em cima e embaixo, no padrão (`TreeViewItemPresenterMargin`).
const MARGIN: (f64, f64) = (4.0, 2.0);

/// Altura de uma linha na densidade `density`.
pub(crate) fn item_height(density: Density) -> f64 {
    match density {
        Density::Standard => TREE_VIEW_ITEM_HEIGHT,
        Density::Compact => TREE_VIEW_ITEM_HEIGHT_COMPACT,
    }
}

/// Diâmetro do selo de estado (`.item_status`), sem o contorno.
pub const TREE_VIEW_STATUS_BADGE: f64 = 10.0;

/// O estado de um item, desenhado num selo no canto do ícone, como o `InfoBadge` do WinUI.
///
/// A forma diz o estado junto com a cor: cheio, anel ou com "!".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemStatus {
    /// Círculo cheio verde (`SystemFillColorSuccess`).
    Success,
    /// Anel âmbar (`SystemFillColorCaution`).
    Caution,
    /// Círculo cheio vermelho com "!" (`SystemFillColorCritical`).
    Critical,
    /// Anel cinza (`TextFillColorTertiary`).
    Neutral,
}

impl ItemStatus {
    /// O estado em palavras, para o nome acessível da linha.
    pub(crate) fn text(self) -> String {
        let strings = crate::strings();
        match self {
            Self::Success => &strings.tree_status_success,
            Self::Caution => &strings.tree_status_caution,
            Self::Critical => &strings.tree_status_critical,
            Self::Neutral => &strings.tree_status_neutral,
        }
        .to_string()
    }

    /// O nome do estado nas ferramentas (`inspect`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Caution => "caution",
            Self::Critical => "critical",
            Self::Neutral => "neutral",
        }
    }
}

/// Onde o arrasto vai soltar em relação a uma linha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropMark {
    /// Dentro da pasta: o fundo da linha ganha o realce.
    Into,
    /// Linha de inserção em cima da linha, recuada ao nível dado.
    Top(usize),
    /// Linha de inserção embaixo da linha, recuada ao nível dado.
    Bottom(usize),
}

/// Tudo o que o `Drawer` precisa saber de uma linha do `TreeView` para pintá-la.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TreeViewItemVisual {
    /// Nível: 0 na raiz.
    pub depth: usize,
    /// Se mostra o chevron (pasta com filhos).
    pub has_children: bool,
    /// Se a pasta aparece aberta.
    pub expanded: bool,
    /// Se é a linha selecionada.
    pub selected: bool,
    /// Estado.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Se o rótulo foi trocado por um campo de texto (o `Edit` é filho da linha).
    pub renaming: bool,
    /// Onde o arrasto vai cair, em relação a esta linha.
    pub drop: Option<DropMark>,
    /// O selo de estado no canto do ícone (`.item_status`).
    pub status: Option<ItemStatus>,
    /// Densidade.
    pub density: Density,
}

impl TreeViewItemVisual {
    /// Altura da linha.
    pub fn height(self) -> f64 {
        item_height(self.density)
    }

    /// O fundo da linha: o retângulo menos a margem (que no compacto é zero).
    pub fn pill(self, rect: Rect) -> Rect {
        let (x, y) = match self.density {
            Density::Standard => MARGIN,
            Density::Compact => (0.0, 0.0),
        };
        Rect::new(
            rect.x0 + x,
            rect.y0 + y,
            (rect.x1 - x).max(rect.x0 + x),
            (rect.y0 + self.height() - y).max(rect.y0 + y),
        )
    }

    /// Começo do conteúdo da linha de nível `depth`: depois do recuo.
    fn content_x(self, rect: Rect, depth: usize) -> f64 {
        self.pill(rect).x0 + TREE_VIEW_INDENT * depth as f64
    }

    /// A coluna do chevron: o clique nela abre ou fecha a pasta.
    pub fn chevron_column(self, rect: Rect) -> Rect {
        let pill = self.pill(rect);
        let x = self.content_x(rect, self.depth);
        Rect::new(x, pill.y0, x + TREE_VIEW_CHEVRON_COLUMN, pill.y1)
    }

    /// O quadrado do ícone.
    pub fn icon_square(self, rect: Rect) -> Rect {
        let pill = self.pill(rect);
        let x = self.chevron_column(rect).x1;
        let y = pill.y0 + ((pill.height() - TREE_VIEW_ICON) / 2.0).floor();
        Rect::new(x, y, x + TREE_VIEW_ICON, y + TREE_VIEW_ICON)
    }

    /// O quadrado do selo de estado: no canto inferior direito do quadrado do ícone, um pouco
    /// para fora dele.
    pub fn status_square(self, rect: Rect) -> Rect {
        let icon = self.icon_square(rect);
        let (x, y) = (
            icon.x1 - TREE_VIEW_STATUS_BADGE + 2.0,
            icon.y1 - TREE_VIEW_STATUS_BADGE + 2.0,
        );
        Rect::new(x, y, x + TREE_VIEW_STATUS_BADGE, y + TREE_VIEW_STATUS_BADGE)
    }

    /// Onde o rótulo começa.
    pub fn label_x(self, rect: Rect) -> f64 {
        self.icon_square(rect).x1 + TREE_VIEW_ICON_GAP
    }

    /// Começo, na horizontal, da linha de inserção do nível `level`: alinhada ao ícone dos
    /// itens desse nível.
    pub fn insert_x(self, rect: Rect, level: usize) -> f64 {
        self.content_x(rect, level) + TREE_VIEW_CHEVRON_COLUMN
    }
}

/// Tudo o que as ferramentas precisam saber do `TreeView`.
#[derive(Clone, Debug)]
pub struct TreeViewVisual {
    /// As linhas visíveis, em ordem.
    pub rows: Arc<TreeFlat>,
    /// A chave da linha selecionada, se aparece.
    pub selected: Option<String>,
    /// O texto do filtro.
    pub filter: String,
}

impl PartialEq for TreeViewVisual {
    fn eq(&self, other: &Self) -> bool {
        self.selected == other.selected
            && self.filter == other.filter
            && (Arc::ptr_eq(&self.rows, &other.rows) || self.rows == other.rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometria_da_linha() {
        let rect = Rect::new(0.0, 100.0, 300.0, 144.0);
        let v = TreeViewItemVisual {
            depth: 1,
            ..TreeViewItemVisual::default()
        };
        assert_eq!(v.pill(rect), Rect::new(4.0, 102.0, 296.0, 142.0));
        assert_eq!(v.chevron_column(rect), Rect::new(20.0, 102.0, 60.0, 142.0));
        assert_eq!(v.icon_square(rect), Rect::new(60.0, 114.0, 76.0, 130.0));
        assert_eq!(v.label_x(rect), 84.0);
        assert_eq!(v.insert_x(rect, 0), 44.0);
        assert_eq!(v.status_square(rect), Rect::new(68.0, 122.0, 78.0, 132.0));
    }

    #[test]
    fn linha_compacta_nao_tem_margem() {
        let rect = Rect::new(0.0, 0.0, 300.0, 24.0);
        let v = TreeViewItemVisual {
            density: Density::Compact,
            ..TreeViewItemVisual::default()
        };
        assert_eq!(v.pill(rect), rect);
        assert_eq!(v.icon_square(rect), Rect::new(40.0, 4.0, 56.0, 20.0));
    }
}
