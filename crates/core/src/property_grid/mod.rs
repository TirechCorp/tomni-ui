//! `PropertyGrid`: lista de propriedades de duas colunas (nome e valor), por categoria ou em
//! ordem alfabética, com um editor por tipo de valor.

mod editor;
mod model;
mod row;
mod shared;
mod sizer;
mod visual;

use std::fmt;
use std::ops::RangeInclusive;
use std::rc::Rc;

use kurbo::Size;
use tomni_ui_reactive::prelude::*;

pub use model::PropertyValue;
use model::{Category, Filter, Kind, Property, RowKey, Source};
pub use visual::{
    CategoryInfo, PROPERTY_GRID_GRIP, PROPERTY_GRID_INDENT, PROPERTY_GRID_MIN_COLUMN,
    PROPERTY_GRID_NAME_X, PropertyGridCategoryVisual, PropertyGridRowVisual,
    PropertyGridSizerVisual, PropertyGridVisual, PropertyInfo,
};

use crate::containers::{Column, Row};
use crate::density::Density;
use crate::prop::{IntoProp, Prop};
use crate::scroll_viewer::ScrollViewer;
use crate::symbol::Symbol;
use crate::text::Text;
use crate::toggle_button::ToggleButton;
use crate::widget::{
    Common, Flex, Insets, Justify, Look, Widget, common_builders, density_builder,
};
use row::GridRows;
use shared::Shared;
use sizer::{DescriptionPane, DescriptionSizer};

/// O último item declarado: os modificadores valem para ele.
#[derive(Clone, Copy)]
enum Last {
    Property(usize),
    Category(usize),
}

/// Lista de propriedades de duas colunas, nome e valor, como o `PropertyGrid` do Windows
/// Forms.
///
/// ```
/// use tomni_ui_core::PropertyGrid;
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let host = signal(String::from("exemplo.com"));
/// let porta = signal(22.0);
/// let grade = PropertyGrid::new()
///     .id("props")
///     .category("Conexão")
///     .text("Host", host)
///     .description("Nome ou endereço do servidor.")
///     .number("Porta", porta, 1.0..=65535.0)
///     .default(22)
///     .grow(1);
/// # });
/// # scope.clear();
/// ```
///
/// Cada propriedade é ligada a um `Signal` do app (ou a uma closure que devolve o signal da
/// conexão escolhida). Os modificadores (`.description`, `.default`, `.disabled`,
/// `.read_only`, `.filter`, `.collapsed`, `.default_with` e `.hidden`) valem para o último item
/// declarado.
///
/// - **Ordem.** Por categoria, na ordem em que o app declarou; as propriedades de antes da
///   primeira `.category` ficam no topo, sem cabeçalho. O botão da barra passa para a ordem
///   alfabética, sem categorias.
/// - **Negrito.** O valor de uma propriedade com `.default` aparece em negrito enquanto
///   difere dele. Sem `.default`, nunca fica em negrito.
/// - **Escondidas.** `.hidden` tira da grade uma linha ou uma categoria, e a categoria sem
///   nenhuma linha visível perde o cabeçalho. Em qualquer ordem. O valor fica no signal do
///   app: esconder e mostrar de novo devolve o que estava lá. `.default_with` dá um padrão
///   que acompanha outro campo.
/// - **Densidade.** A grade vem na densidade compacta (linhas de 24 px). Com
///   `.density(Density::Standard)` as linhas têm 44 px.
/// - **Editores.** Só a linha escolhida (e habilitada, e que não é `.read_only`) troca o valor
///   pelo editor do tipo: `Edit` (`.text`), `Edit` com máscara (`.password`), `NumberBox`
///   (`.number`, com os limites do `range`), `ComboBox` (`.choice`) e `Edit` com o botão "…"
///   (`.file`). O `.button` mostra o resumo num `Edit` que não aceita digitação, com o "…" ao
///   lado, e o clique, o Enter, o F2 e o espaço chamam o `on_click`. A `.check` fica à vista em toda linha. O texto, a senha, o número e a `.file`
///   são um rascunho: vão para o signal do app ao confirmar (Enter, Tab ou perder o foco), e o
///   Esc desfaz; mudar o signal por código troca o rascunho. A `.choice` e a `.check` gravam
///   direto no signal.
/// - **Teclado.** ↑/↓ andam pelas linhas (pulando as desabilitadas e as categorias fechadas),
///   ← fecha a categoria ou sobe ao cabeçalho, → abre, Home/End. Enter ou F2 entram no editor
///   (na `.check`, invertem; na `.choice`, abrem a lista, como F4 e Alt+↓); o Esc devolve o
///   foco à linha (no `.button`, Enter, F2 e espaço chamam o `on_click`). Digitar numa linha de texto, senha ou arquivo abre o editor com o texto
///   (na `.number`, só Enter, F2 ou o clique abrem).
/// - **Divisores.** Arrastar a linha entre o nome e o valor muda a largura do nome (mínimo de
///   40 px de cada lado); arrastar a faixa acima da descrição muda a altura do painel.
/// - **Descrição.** O painel de baixo mostra o nome e a `.description` da linha escolhida; no
///   cabeçalho de uma categoria, só o nome dela.
/// - **Seletor de arquivo.** O "…" da `.file` abre o seletor do sistema (`dialogs::open_file`)
///   com o nome da propriedade no título, o `.filter` e a pasta do valor atual; escolher um
///   arquivo grava o caminho e cancelar não muda nada. No Android o botão não aparece: a
///   pessoa digita o caminho.
///
/// # Adaptações ao WinUI 3
///
/// O WinUI 3 não tem `PropertyGrid`; o desenho segue o `TreeView` e os campos dele. O clique
/// duplo no valor de uma `.choice` não avança a opção (o `ComboBox` cobre o valor da linha
/// escolhida: o primeiro clique escolhe a linha e o segundo abre a lista), e o arrasto do
/// divisor das colunas não pega nos 3 px à direita da linha escolhida, que o editor cobre. O
/// `.hidden` também não vem do WinUI 3: segue a grade do Windows Forms, que esconde a propriedade
/// e o cabeçalho que ficou vazio, só que reativo.
pub struct PropertyGrid {
    common: Common,
    shared: Rc<Shared>,
    last: Option<Last>,
}

impl PropertyGrid {
    /// Uma grade sem propriedades.
    pub fn new() -> Self {
        Self {
            common: Common::default(),
            shared: Rc::new(Shared::new()),
            last: None,
        }
    }

    /// Começa uma categoria: as propriedades declaradas a seguir entram nela.
    pub fn category(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        let mut categories = self.shared.categories.borrow_mut();
        debug_assert!(
            !categories.iter().any(|c| c.name == name),
            "categoria repetida: {name}"
        );
        categories.push(Category {
            name,
            collapsed: false,
            hidden: Prop::Value(false),
        });
        self.last = Some(Last::Category(categories.len() - 1));
        drop(categories);
        self
    }

    fn add(mut self, name: String, kind: Kind, source: Source) -> Self {
        let category = self.shared.categories.borrow().len().checked_sub(1);
        let mut properties = self.shared.properties.borrow_mut();
        debug_assert!(
            !properties.iter().any(|p| p.name == name),
            "propriedade repetida: {name}"
        );
        properties.push(Property::new(name, category, kind, source));
        self.last = Some(Last::Property(properties.len() - 1));
        drop(properties);
        self
    }

    /// Uma propriedade de texto.
    pub fn text<M>(self, name: impl Into<String>, value: impl IntoProp<Signal<String>, M>) -> Self {
        self.add(name.into(), Kind::Text, Source::Text(value.into_prop()))
    }

    /// Uma propriedade de texto que aparece mascarada.
    pub fn password<M>(
        self,
        name: impl Into<String>,
        value: impl IntoProp<Signal<String>, M>,
    ) -> Self {
        self.add(name.into(), Kind::Password, Source::Text(value.into_prop()))
    }

    /// Uma propriedade numérica, dentro de `range`.
    pub fn number<M>(
        self,
        name: impl Into<String>,
        value: impl IntoProp<Signal<f64>, M>,
        range: RangeInclusive<f64>,
    ) -> Self {
        let kind = Kind::Number {
            min: *range.start(),
            max: *range.end(),
        };
        self.add(name.into(), kind, Source::Number(value.into_prop()))
    }

    /// Uma propriedade que escolhe uma entre `options`.
    pub fn choice<M, S: Into<String>>(
        self,
        name: impl Into<String>,
        value: impl IntoProp<Signal<String>, M>,
        options: impl IntoIterator<Item = S>,
    ) -> Self {
        let options: Vec<String> = options.into_iter().map(Into::into).collect();
        self.choice_from(name, value, move || options.clone())
    }

    /// Como [`PropertyGrid::choice`], com as opções calculadas por `options`. Serve para uma
    /// lista que muda com o app (os itens de um cofre, os arquivos de uma pasta): `options` pode
    /// ler signals, e o editor da linha se refaz quando a lista muda.
    pub fn choice_from<M>(
        self,
        name: impl Into<String>,
        value: impl IntoProp<Signal<String>, M>,
        options: impl Fn() -> Vec<String> + 'static,
    ) -> Self {
        self.add(
            name.into(),
            Kind::Choice(Rc::new(options)),
            Source::Text(value.into_prop()),
        )
    }

    /// Uma propriedade ligada/desligada.
    pub fn check<M>(self, name: impl Into<String>, value: impl IntoProp<Signal<bool>, M>) -> Self {
        self.add(name.into(), Kind::Check, Source::Bool(value.into_prop()))
    }

    /// Uma propriedade com o caminho de um arquivo, que o botão "…" escolhe no seletor do
    /// sistema.
    pub fn file<M>(self, name: impl Into<String>, value: impl IntoProp<Signal<String>, M>) -> Self {
        self.add(
            name.into(),
            Kind::File(Vec::new()),
            Source::Text(value.into_prop()),
        )
    }

    /// Uma propriedade com o caminho de uma pasta, que o botão "…" escolhe no seletor do
    /// sistema. É a linha da [`PropertyGrid::file`] com o seletor de pasta: não tem `.filter`,
    /// e cancelar o seletor não muda o valor.
    pub fn folder<M>(
        self,
        name: impl Into<String>,
        value: impl IntoProp<Signal<String>, M>,
    ) -> Self {
        self.add(name.into(), Kind::Folder, Source::Text(value.into_prop()))
    }

    /// Uma propriedade só de leitura com um botão "…" que chama `on_click`: o resumo de uma
    /// coleção ("2 túneis") que se edita num diálogo, como o editor "(Coleção)" do Visual
    /// Studio.
    ///
    /// A linha é como a da [`PropertyGrid::file`], com o `summary` no lugar do caminho e sem
    /// edição: o texto não aceita digitação, e o clique no "…", o Enter, o F2 e o espaço (na
    /// linha ou no texto) chamam `on_click`. `.description`, `.disabled` e `.category` valem; o
    /// `.default` não, porque o botão não guarda valor.
    pub fn button<M>(
        self,
        name: impl Into<String>,
        summary: impl IntoProp<Signal<String>, M>,
        on_click: impl Fn() + 'static,
    ) -> Self {
        self.add(
            name.into(),
            Kind::Button(Rc::new(on_click)),
            Source::Text(summary.into_prop()),
        )
    }

    /// Edita a última propriedade declarada; em qualquer outro item, avisa em debug.
    fn on_property(self, what: &str, edit: impl FnOnce(&mut Property)) -> Self {
        match self.last {
            Some(Last::Property(i)) => edit(&mut self.shared.properties.borrow_mut()[i]),
            _ => debug_assert!(
                false,
                "`{what}` vale para uma propriedade, não para o item anterior"
            ),
        }
        self
    }

    /// O texto do painel de descrição quando a propriedade está escolhida.
    pub fn description(self, text: impl Into<String>) -> Self {
        self.on_property(".description", |p| p.description = text.into())
    }

    /// O valor padrão: enquanto o valor atual difere dele, a linha aparece em negrito.
    /// Aceita `&str`, `String`, `f64`, `i32` e `bool`, e o tipo precisa combinar com a
    /// propriedade.
    pub fn default(self, value: impl Into<PropertyValue>) -> Self {
        self.on_property(".default", |p| p.set_default(value.into()))
    }

    /// O valor padrão calculado: como [`PropertyGrid::default`], mas `f` é lida de forma
    /// rastreada, e o padrão acompanha outro campo (a porta padrão de cada tipo de conexão). O
    /// tipo devolvido precisa combinar com a propriedade (avisa em debug).
    ///
    /// ```
    /// use tomni_ui_core::PropertyGrid;
    /// use tomni_ui_reactive::prelude::*;
    ///
    /// # let scope = Scope::root();
    /// # scope.run(|| {
    /// let tipo = signal(String::from("SSH"));
    /// let porta = signal(22.0);
    /// let grade = PropertyGrid::new()
    ///     .number("Porta", porta, 1.0..=65535.0)
    ///     .default_with(move || if tipo.get() == "Telnet" { 23 } else { 22 });
    /// # });
    /// # scope.clear();
    /// ```
    pub fn default_with<V: Into<PropertyValue>>(self, f: impl Fn() -> V + 'static) -> Self {
        self.on_property(".default_with", |p| {
            p.set_default_with(Rc::new(move || f().into()));
        })
    }

    /// Esconde a linha, ou, depois de uma [`PropertyGrid::category`], a categoria inteira
    /// (cabeçalho e linhas). Aceita o mesmo que [`PropertyGrid::disabled`]: um valor fixo, uma
    /// closure ou um signal.
    ///
    /// A linha escondida sai das linhas: não recebe foco nem Tab, as setas passam por ela, e
    /// ela some da árvore de acessibilidade e das ferramentas (que a listam com `hidden`).
    /// Nada se apaga: o valor continua no signal do app, e a linha volta com ele. Uma categoria
    /// sem nenhuma linha visível também perde o cabeçalho, sem precisar de `.hidden` nela. Se
    /// a linha escolhida some, a escolha fica sem linha até outra ser escolhida.
    ///
    /// Esconda o que nunca vale; [`PropertyGrid::disabled`] é para o que depende de outro
    /// campo.
    ///
    /// ```
    /// use tomni_ui_core::PropertyGrid;
    /// use tomni_ui_reactive::prelude::*;
    ///
    /// # let scope = Scope::root();
    /// # scope.run(|| {
    /// let proxy = signal(false);
    /// let endereco = signal(String::new());
    /// let grade = PropertyGrid::new()
    ///     .check("Usar proxy", proxy)
    ///     .text("Endereço do proxy", endereco)
    ///     .hidden(move || !proxy.get());
    /// # });
    /// # scope.clear();
    /// ```
    pub fn hidden<M>(self, hidden: impl IntoProp<bool, M>) -> Self {
        match self.last {
            Some(Last::Property(i)) => {
                self.shared.properties.borrow_mut()[i].hidden = hidden.into_prop();
            }
            Some(Last::Category(c)) => {
                self.shared.categories.borrow_mut()[c].hidden = hidden.into_prop();
            }
            None => debug_assert!(
                false,
                "`.hidden` vale para uma propriedade ou uma categoria"
            ),
        }
        self
    }

    /// Desabilita a linha (valor fixo, closure ou signal): ela não recebe foco e as setas
    /// passam por ela.
    pub fn disabled<M>(self, disabled: impl IntoProp<bool, M>) -> Self {
        self.on_property(".disabled", |p| p.disabled = disabled.into_prop())
    }

    /// Só leitura: sem editor, o valor aparece em cinza. A linha ainda pode ser escolhida,
    /// para ler a descrição.
    pub fn read_only(self) -> Self {
        self.on_property(".read_only", |p| p.read_only = true)
    }

    /// Um filtro do seletor de arquivo de uma `.file`: `.filter("Chaves", &["pem", "key"])`.
    pub fn filter(self, name: impl Into<String>, extensions: &[&str]) -> Self {
        let name = name.into();
        self.on_property(".filter", |p| match &mut p.kind {
            Kind::File(filters) => filters.push(Filter {
                name,
                extensions: extensions.iter().map(|e| (*e).to_owned()).collect(),
            }),
            _ => debug_assert!(false, "`.filter` vale só para `.file`"),
        })
    }

    /// A categoria começa fechada.
    pub fn collapsed(self) -> Self {
        match self.last {
            Some(Last::Category(c)) => {
                let name = {
                    let mut categories = self.shared.categories.borrow_mut();
                    categories[c].collapsed = true;
                    categories[c].name.clone()
                };
                self.shared.collapsed.update(|set| {
                    set.insert(name);
                });
            }
            _ => debug_assert!(false, "`.collapsed` vale só para uma categoria"),
        }
        self
    }
}

impl Default for PropertyGrid {
    fn default() -> Self {
        Self::new()
    }
}

common_builders!(PropertyGrid);
density_builder!(PropertyGrid);

impl Widget for PropertyGrid {
    fn kind(&self) -> &'static str {
        "property_grid"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn names_children(&self) -> bool {
        true
    }

    fn density(&self) -> Option<Density> {
        self.common.density.or(Some(Density::Compact))
    }

    fn set_density(&mut self, density: Density) {
        self.shared.density.set(density);
    }

    fn look(&self) -> Look {
        Look::PropertyGrid(self.shared.visual())
    }

    fn flex(&self) -> Option<Flex> {
        Some(Flex::default())
    }

    fn resized(&mut self, size: Size) {
        let shared = &self.shared;
        if untrack(|| shared.width.get()) != size.width {
            shared.width.set(size.width);
        }
        if untrack(|| shared.height.get()) != size.height {
            shared.height.set(size.height);
            // A grade menor pode não comportar mais a altura da descrição.
            shared.set_description_height(untrack(|| shared.description_height.get()));
        }
    }

    fn take_children(&mut self) -> Vec<Box<dyn Widget>> {
        let shared = &self.shared;
        *shared.id.borrow_mut() = self.common.id.clone();
        // A primeira propriedade habilitada começa escolhida, para a descrição não ficar vazia.
        if shared.visible_selected().is_none() {
            let first = shared
                .rows_now()
                .into_iter()
                .find(|key| matches!(key, RowKey::Property(_)) && shared.enabled(*key));
            if let Some(first) = first {
                shared.select(first);
            }
        }
        let mut bar = Row::new().padding(4).justify(Justify::End).child(
            ToggleButton::icon_only(shared.alphabetical, Symbol::ArrowSort)
                .label(crate::strings().sort_alphabetical.to_string())
                .generated_id(shared.sub("alfabetica")),
        );
        let mut scroll = ScrollViewer::new()
            .generated()
            .grow(1)
            .child(Column::new().child(GridRows::new(shared)));
        if let Some(id) = shared.sub("barra") {
            bar = bar.id(id);
        }
        if let Some(id) = shared.sub("rolagem") {
            scroll = scroll.id(id);
        }
        let (name, text) = (Rc::clone(shared), Rc::clone(shared));
        let mut title = Text::new(move || name.description().0).body_strong();
        let mut body = Text::new(move || text.description().1).wrap().secondary();
        if let Some(id) = shared.sub("descricao-nome") {
            title = title.id(id);
        }
        if let Some(id) = shared.sub("descricao-texto") {
            body = body.id(id);
        }
        let mut description = ScrollViewer::new().generated().child(
            Column::new()
                .padding(Insets::symmetric(8.0, 12.0))
                .gap(2)
                .child(title)
                .child(body),
        );
        if let Some(id) = shared.sub("descricao") {
            description = description.id(id);
        }
        vec![
            Box::new(bar),
            Box::new(scroll),
            Box::new(DescriptionSizer::new(shared)),
            Box::new(DescriptionPane::new(shared, Box::new(description))),
        ]
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl fmt::Debug for PropertyGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PropertyGrid")
            .field("common", &self.common)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "propriedade repetida")]
    fn nome_repetido_da_pânico_em_debug() {
        let scope = Scope::root();
        scope.run(|| {
            let a = signal(String::new());
            let _ = PropertyGrid::new().text("Host", a).text("Host", a);
        });
    }

    #[test]
    fn modificadores_valem_para_o_ultimo_item() {
        let scope = Scope::root();
        scope.run(|| {
            let host = signal(String::new());
            let grade = PropertyGrid::new()
                .text("Antes", host)
                .category("Conexão")
                .collapsed()
                .text("Host", host)
                .description("Nome do servidor.")
                .default("")
                .read_only();
            let propriedades = grade.shared.properties.borrow();
            assert_eq!(propriedades[0].category, None);
            assert_eq!(propriedades[1].category, Some(0));
            assert_eq!(propriedades[1].description, "Nome do servidor.");
            assert!(propriedades[1].read_only);
            assert!(propriedades[1].default.is_some());
            assert!(propriedades[0].default.is_none());
            assert!(untrack(|| grade.shared.collapsed.get()).contains("Conexão"));
        });
        scope.clear();
    }
}
