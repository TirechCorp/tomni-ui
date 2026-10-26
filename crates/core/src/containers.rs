//! `Form`, `Column`, `Row` e `UniformGrid`: os widgets que só organizam outros.

use std::fmt;

use crate::prop::{IntoProp, Prop};
use crate::title_bar::TitleBar;
use crate::widget::{
    Align, Common, Direction, Flex, Insets, Justify, Look, View, Widget, common_builders,
    density_builder,
};

/// Tela de um app: pinta o fundo do tema e dá ao filho a janela inteira, menos a safe area.
///
/// ```
/// use tomni_ui_core::{Button, Column, Form};
///
/// let tela = Form::new("Login").child(
///     Column::new()
///         .gap(8)
///         .padding(16)
///         .child(Button::new("Entrar").id("entrar").accent()),
/// );
/// ```
pub struct Form {
    common: Common,
    title: Prop<String>,
    title_bar: Option<Box<dyn Widget>>,
    child: Option<Box<dyn Widget>>,
}

impl Form {
    /// Tela com o título `title` (texto fixo, closure ou signal), mostrado na janela.
    pub fn new<M>(title: impl IntoProp<String, M>) -> Self {
        Self {
            common: Common::default(),
            title: title.into_prop(),
            title_bar: None,
            child: None,
        }
    }

    /// Barra de título do app, em cima do conteúdo, no lugar da do sistema: a janela perde a
    /// moldura, e a barra desenha os botões da janela, arrasta e redimensiona. Onde a
    /// plataforma não deixa (Android, ou um Wayland que recusa o arrasto), a barra do sistema
    /// fica e a do app aparece embaixo dela, sem os botões da janela.
    pub fn title_bar(mut self, title_bar: TitleBar) -> Self {
        self.title_bar = Some(Box::new(title_bar));
        self
    }

    /// Conteúdo da tela. Chamar de novo troca o anterior.
    pub fn child(mut self, child: impl View) -> Self {
        self.child = Some(Box::new(child));
        self
    }
}

common_builders!(Form);
density_builder!(Form);

impl Widget for Form {
    fn kind(&self) -> &'static str {
        "form"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Form
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex {
            fill: true,
            ..Flex::default()
        })
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        self.title_bar
            .take()
            .into_iter()
            .chain(self.child.take())
            .collect()
    }

    fn title(&self) -> Option<String> {
        Some(self.title.get())
    }
}

impl fmt::Debug for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Form")
            .field("common", &self.common)
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

/// Define um contêiner flex com os builders de `gap`, `padding`, `align`, `justify` e `child`.
macro_rules! container {
    ($(#[$doc:meta])* $name:ident, $kind:literal, $direction:expr, $look:expr) => {
        $(#[$doc])*
        pub struct $name {
            common: Common,
            flex: Flex,
            children: Vec<Box<dyn Widget>>,
        }

        impl $name {
            /// Contêiner vazio, sem `gap` nem `padding`.
            pub fn new() -> Self {
                Self {
                    common: Common::default(),
                    flex: Flex {
                        direction: $direction,
                        ..Flex::default()
                    },
                    children: Vec::new(),
                }
            }

            /// Espaço entre filhos vizinhos, em pixels.
            pub fn gap(mut self, gap: impl Into<f64>) -> Self {
                self.flex.gap = gap.into();
                self
            }

            /// Espaço entre a borda e os filhos: um número para os quatro lados ou
            /// [`Insets`].
            pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
                self.flex.padding = padding.into();
                self
            }

            /// Alinhamento dos filhos no eixo cruzado. O padrão é [`Align::Stretch`].
            pub fn align(mut self, align: Align) -> Self {
                self.flex.align = align;
                self
            }

            /// Distribuição dos filhos no eixo principal. O padrão é [`Justify::Start`].
            pub fn justify(mut self, justify: Justify) -> Self {
                self.flex.justify = justify;
                self
            }

            /// Acrescenta um filho no fim.
            pub fn child(mut self, child: impl View) -> Self {
                self.children.push(Box::new(child));
                self
            }

            /// Acrescenta um filho já em caixa no fim.
            #[allow(dead_code, reason = "nem todo contêiner recebe filhos em caixa")]
            pub(crate) fn boxed(mut self, child: Box<dyn Widget>) -> Self {
                self.children.push(child);
                self
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        common_builders!($name);
        density_builder!($name);

        impl Widget for $name {
            fn kind(&self) -> &'static str {
                $kind
            }

            fn common(&self) -> &Common {
                &self.common
            }

            fn look(&self) -> Look {
                $look
            }

            fn flex(&self) -> Option<Flex> {
                Some(self.flex)
            }

            fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
                std::mem::take(&mut self.children)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("common", &self.common)
                    .field("flex", &self.flex)
                    .field("children", &self.children.len())
                    .finish()
            }
        }
    };
}

container!(
    /// Empilha os filhos de cima para baixo.
    ///
    /// ```
    /// use tomni_ui_core::{Align, Button, Column};
    ///
    /// let coluna = Column::new()
    ///     .gap(8)
    ///     .padding(16)
    ///     .align(Align::Center)
    ///     .child(Button::new("Um"))
    ///     .child(Button::new("Dois"));
    /// ```
    Column,
    "column",
    Direction::Column,
    Look::Column
);

container!(
    /// Põe os filhos lado a lado, da esquerda para a direita.
    ///
    /// ```
    /// use tomni_ui_core::{Button, Justify, Row};
    ///
    /// let botoes = Row::new()
    ///     .gap(8)
    ///     .justify(Justify::End)
    ///     .child(Button::new("Cancelar"))
    ///     .child(Button::new("Salvar").accent());
    /// ```
    Row,
    "row",
    Direction::Row,
    Look::Row
);

impl Row {
    /// Passa para a linha de baixo os filhos que não cabem, como o `flex-wrap` do CSS. O
    /// `gap` vale também entre as linhas, e nenhum filho é cortado nem encolhe abaixo do
    /// tamanho natural.
    ///
    /// ```
    /// use tomni_ui_core::{Button, Row};
    ///
    /// let etiquetas = Row::new()
    ///     .wrap()
    ///     .gap(8)
    ///     .child(Button::new("Rust"))
    ///     .child(Button::new("Interface"))
    ///     .child(Button::new("Fluent"));
    /// ```
    pub fn wrap(mut self) -> Self {
        self.flex.wrap = true;
        self
    }
}

/// Largura mínima das colunas de um `UniformGrid` sem `.min_column_width`.
pub const UNIFORM_GRID_MIN_COLUMN_WIDTH: f64 = 200.0;

/// Grade de colunas iguais: em quantas colunas de pelo menos `.min_column_width` couberem na
/// largura, com os filhos da mesma largura e, em todas as linhas, da mesma altura (a do mais
/// alto), como o `UniformGridLayout` do `ItemsRepeater` do WinUI com `ItemsStretch="Fill"`.
///
/// ```
/// use tomni_ui_core::{Card, Text, UniformGrid};
///
/// let grade = UniformGrid::new()
///     .min_column_width(240)
///     .gap(12)
///     .child(Card::new().child(Text::new("Um")))
///     .child(Card::new().child(Text::new("Dois")))
///     .child(Card::new().child(Text::new("Três")));
/// ```
///
/// Os filhos entram da esquerda para a direita e descem quando a linha enche. Com menos filhos
/// que colunas, as colunas que sobram ficam vazias: os filhos não esticam para ocupá-las.
pub struct UniformGrid {
    common: Common,
    flex: Flex,
    children: Vec<Box<dyn Widget>>,
}

impl UniformGrid {
    /// Grade vazia, com colunas de pelo menos [`UNIFORM_GRID_MIN_COLUMN_WIDTH`] px, sem `gap`
    /// nem `padding`.
    pub fn new() -> Self {
        Self {
            common: Common::default(),
            flex: Flex {
                min_column_width: Some(UNIFORM_GRID_MIN_COLUMN_WIDTH),
                ..Flex::default()
            },
            children: Vec::new(),
        }
    }

    /// Largura mínima de cada coluna, em pixels: cabem tantas colunas quantas couberem com
    /// essa largura e o `gap` entre elas, e as colunas dividem a sobra igualmente.
    pub fn min_column_width(mut self, width: impl Into<f64>) -> Self {
        self.flex.min_column_width = Some(width.into().max(1.0));
        self
    }

    /// Espaço entre as colunas e entre as linhas, em pixels.
    pub fn gap(mut self, gap: impl Into<f64>) -> Self {
        self.flex.gap = gap.into();
        self
    }

    /// Espaço entre a borda e os filhos: um número para os quatro lados ou [`Insets`].
    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.flex.padding = padding.into();
        self
    }

    /// Acrescenta um filho no fim.
    pub fn child(mut self, child: impl View) -> Self {
        self.children.push(Box::new(child));
        self
    }
}

impl Default for UniformGrid {
    fn default() -> Self {
        Self::new()
    }
}

common_builders!(UniformGrid);
density_builder!(UniformGrid);

impl Widget for UniformGrid {
    fn kind(&self) -> &'static str {
        "uniform_grid"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Column
    }

    fn flex(&self) -> Option<Flex> {
        Some(self.flex)
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        std::mem::take(&mut self.children)
    }
}

impl fmt::Debug for UniformGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UniformGrid")
            .field("common", &self.common)
            .field("flex", &self.flex)
            .field("children", &self.children.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Button;

    #[test]
    fn builders() {
        let mut coluna = Column::new()
            .id("lista")
            .gap(8)
            .padding(Insets::symmetric(4.0, 12.0))
            .align(Align::Center)
            .justify(Justify::SpaceBetween)
            .grow(1)
            .width(200)
            .child(Button::new("Um"))
            .child(Button::new("Dois"));
        assert_eq!(coluna.common().id.as_deref(), Some("lista"));
        assert_eq!(coluna.common().grow, 1.0);
        assert_eq!(coluna.common().width, Some(200.0));
        assert_eq!(
            coluna.flex(),
            Some(Flex {
                direction: Direction::Column,
                gap: 8.0,
                padding: Insets::new(4.0, 12.0, 4.0, 12.0),
                align: Align::Center,
                justify: Justify::SpaceBetween,
                fill: false,
                wrap: false,
                min_column_width: None,
            })
        );
        assert_eq!(coluna.take_children().len(), 2);
        assert!(
            coluna.take_children().is_empty(),
            "os filhos saem uma vez só"
        );
        assert_eq!(Row::new().flex().map(|f| f.direction), Some(Direction::Row));
        assert!(Row::new().wrap().flex().is_some_and(|f| f.wrap));
    }

    #[test]
    fn grade_uniforme() {
        let mut grade = UniformGrid::new()
            .min_column_width(240)
            .gap(12)
            .child(Button::new("Um"));
        let flex = grade.flex().unwrap();
        assert_eq!(flex.min_column_width, Some(240.0));
        assert_eq!(flex.gap, 12.0);
        assert_eq!(grade.kind(), "uniform_grid");
        assert_eq!(grade.take_children().len(), 1);
    }

    #[test]
    fn form_preenche_e_tem_titulo() {
        let mut form = Form::new("Login").child(Column::new());
        assert_eq!(form.title().as_deref(), Some("Login"));
        assert!(form.flex().is_some_and(|f| f.fill));
        assert_eq!(form.take_children().len(), 1);
    }
}
