//! O que o `Drawer` e as ferramentas precisam saber da `PropertyGrid`, das linhas dela e do
//! divisor da descrição, e a geometria que o widget (os cliques) e o tema (o desenho) dividem.

use kurbo::Rect;

use crate::button::ButtonState;
use crate::density::Density;
use crate::tree_view::item_height;

/// Onde o nome começa, à esquerda da linha.
pub const PROPERTY_GRID_NAME_X: f64 = 12.0;
/// Recuo do nome das propriedades dentro de uma categoria (na ordem por categoria).
pub const PROPERTY_GRID_INDENT: f64 = 16.0;
/// Até onde, em px, o ponteiro "pega" a linha das colunas.
pub const PROPERTY_GRID_GRIP: f64 = 3.0;
/// Menor largura de cada coluna, em px.
pub const PROPERTY_GRID_MIN_COLUMN: f64 = 40.0;

/// Uma propriedade, como as ferramentas a veem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyInfo {
    /// A categoria; `None` antes da primeira.
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
    /// Se não aparece: escondida por `.hidden`, pela categoria dela ou porque a categoria
    /// inteira está escondida. O valor continua guardado.
    pub hidden: bool,
}

/// Uma categoria, como as ferramentas a veem.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryInfo {
    /// O nome.
    pub name: String,
    /// Se está aberta.
    pub expanded: bool,
    /// Se não aparece: escondida por `.hidden` ou sem nenhuma linha visível.
    pub hidden: bool,
}

/// Tudo o que as ferramentas precisam saber da `PropertyGrid`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PropertyGridVisual {
    /// Todas as propriedades, na ordem da declaração.
    pub properties: Vec<PropertyInfo>,
    /// As categorias, na ordem da declaração.
    pub categories: Vec<CategoryInfo>,
    /// Se está na ordem alfabética.
    pub alphabetical: bool,
}

/// Tudo o que o `Drawer` precisa saber de uma linha de propriedade.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PropertyGridRowVisual {
    /// Se o valor difere do `.default` (o valor sai em negrito).
    pub changed: bool,
    /// Se é só de leitura (o valor sai em cinza).
    pub read_only: bool,
    /// Se a linha está desabilitada.
    pub disabled: bool,
    /// Se é a linha escolhida.
    pub selected: bool,
    /// Se a linha tem um editor no lugar do valor.
    pub editing: bool,
    /// Se o nome é recuado (ordem por categoria).
    pub indent: bool,
    /// Largura da coluna do nome.
    pub name_width: f64,
    /// Estado.
    pub state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Densidade.
    pub density: Density,
}

impl PropertyGridRowVisual {
    /// Altura da linha.
    pub fn height(self) -> f64 {
        item_height(self.density)
    }

    /// Onde o nome começa.
    pub fn name_x(self, rect: Rect) -> f64 {
        let indent = if self.indent {
            PROPERTY_GRID_INDENT
        } else {
            0.0
        };
        rect.x0 + PROPERTY_GRID_NAME_X + indent
    }

    /// x da linha que divide as colunas.
    pub fn divider_x(self, rect: Rect) -> f64 {
        rect.x0 + self.name_width
    }

    /// Se `x` está perto o bastante da linha das colunas para arrastá-la.
    pub fn near_divider(self, rect: Rect, x: f64) -> bool {
        (x - self.divider_x(rect)).abs() <= PROPERTY_GRID_GRIP
    }
}

/// Tudo o que o `Drawer` precisa saber do cabeçalho de uma categoria.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PropertyGridCategoryVisual {
    /// Se a categoria está aberta.
    pub expanded: bool,
    /// Se é a linha escolhida.
    pub selected: bool,
    /// Estado.
    pub state: ButtonState,
    /// Estado do chevron: hover quando o ponteiro está no quadrado dele.
    pub chevron_state: ButtonState,
    /// Anel de foco: só com foco vindo do teclado.
    pub focus_ring: bool,
    /// Densidade.
    pub density: Density,
}

impl PropertyGridCategoryVisual {
    /// Altura do cabeçalho.
    pub fn height(self) -> f64 {
        item_height(self.density)
    }

    /// O quadrado do chevron, encostado à direita: o clique nele abre ou fecha.
    pub fn chevron_square(self, rect: Rect) -> Rect {
        let side = self.height().min(rect.width().max(0.0));
        Rect::new(rect.x1 - side, rect.y0, rect.x1, rect.y0 + self.height())
    }
}

/// O divisor da descrição.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PropertyGridSizerVisual {
    /// Estado: hover ou arrastando.
    pub state: ButtonState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometria_da_linha() {
        let rect = Rect::new(10.0, 0.0, 310.0, 24.0);
        let v = PropertyGridRowVisual {
            name_width: 150.0,
            indent: true,
            ..PropertyGridRowVisual::default()
        };
        assert_eq!(v.name_x(rect), 38.0);
        assert_eq!(v.divider_x(rect), 160.0);
        assert!(v.near_divider(rect, 163.0));
        assert!(!v.near_divider(rect, 164.0));
    }

    #[test]
    fn chevron_do_cabecalho_e_um_quadrado_da_altura_da_linha() {
        let rect = Rect::new(0.0, 100.0, 300.0, 124.0);
        let v = PropertyGridCategoryVisual {
            density: Density::Compact,
            ..PropertyGridCategoryVisual::default()
        };
        assert_eq!(
            v.chevron_square(rect),
            Rect::new(276.0, 100.0, 300.0, 124.0)
        );
    }
}
