//! O modelo da `PropertyGrid`: as propriedades, as categorias e as linhas visíveis. Nada aqui
//! depende da árvore de widgets.

use std::collections::BTreeSet;
use std::rc::Rc;

use tomni_ui_reactive::{Get, Signal, untrack};

use crate::edit::PASSWORD_CHAR;
use crate::number_box::format_display;
use crate::prop::Prop;
use crate::tree_view::fold;

/// O valor de uma propriedade, no `.default` e nas ferramentas.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// Texto: `.text`, `.password`, `.choice`, `.file` e o resumo do `.button`.
    Text(String),
    /// Número: `.number`.
    Number(f64),
    /// Marcação: `.check`.
    Bool(bool),
}

impl From<&str> for PropertyValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for PropertyValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<f64> for PropertyValue {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<i32> for PropertyValue {
    fn from(value: i32) -> Self {
        Self::Number(value.into())
    }
}

impl From<bool> for PropertyValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

/// O signal ligado a uma propriedade: fixo ou devolvido por uma closure (a conexão escolhida).
pub(crate) enum Source {
    Text(Prop<Signal<String>>),
    Number(Prop<Signal<f64>>),
    Bool(Prop<Signal<bool>>),
}

/// Um filtro do seletor de arquivo da `.file`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Filter {
    pub name: String,
    pub extensions: Vec<String>,
}

/// O tipo da propriedade, que decide o editor.
pub(crate) enum Kind {
    Text,
    Password,
    Number {
        min: f64,
        max: f64,
    },
    /// As opções, calculadas cada vez que o editor abre.
    Choice(Rc<dyn Fn() -> Vec<String>>),
    Check,
    File(Vec<Filter>),
    /// Como `File`, mas o botão "…" escolhe uma pasta.
    Folder,
    /// Só leitura, com o botão "…" que chama a closure (a coleção que um diálogo edita).
    Button(Rc<dyn Fn()>),
}

impl Kind {
    /// Se o valor `default` combina com o tipo.
    fn accepts(&self, value: &PropertyValue) -> bool {
        matches!(
            (self, value),
            (
                Self::Text | Self::Password | Self::Choice(_) | Self::File(_) | Self::Folder,
                PropertyValue::Text(_)
            ) | (Self::Number { .. }, PropertyValue::Number(_))
                | (Self::Check, PropertyValue::Bool(_))
        )
    }
}

/// O padrão de uma propriedade: fixo (`.default`) ou calculado (`.default_with`).
pub(crate) enum PropertyDefault {
    Fixed(PropertyValue),
    /// Lido de forma rastreada, como o valor.
    With(Rc<dyn Fn() -> PropertyValue>),
}

/// Uma propriedade da grade.
pub(crate) struct Property {
    pub name: String,
    /// O índice da categoria; `None` antes da primeira `.category`.
    pub category: Option<usize>,
    pub kind: Kind,
    pub source: Source,
    pub description: String,
    pub default: Option<PropertyDefault>,
    pub disabled: Prop<bool>,
    /// Se a linha não aparece (o valor fica guardado).
    pub hidden: Prop<bool>,
    pub read_only: bool,
}

impl Property {
    pub fn new(name: String, category: Option<usize>, kind: Kind, source: Source) -> Self {
        Self {
            name,
            category,
            kind,
            source,
            description: String::new(),
            default: None,
            disabled: Prop::Value(false),
            hidden: Prop::Value(false),
            read_only: false,
        }
    }

    /// Define o `.default`; um tipo que não combina com a propriedade é ignorado, com aviso em
    /// debug.
    pub fn set_default(&mut self, value: PropertyValue) {
        let ok = self.kind.accepts(&value);
        debug_assert!(
            ok,
            "o `.default` de \"{}\" não combina com o tipo da propriedade",
            self.name
        );
        if ok {
            self.default = Some(PropertyDefault::Fixed(value));
        }
    }

    /// Define o `.default_with`: o padrão calculado, lido de forma rastreada.
    pub fn set_default_with(&mut self, f: Rc<dyn Fn() -> PropertyValue>) {
        self.default = Some(PropertyDefault::With(f));
    }

    /// O padrão agora (rastreado, se calculado). Um padrão calculado de outro tipo avisa em
    /// debug e vale como ausente.
    pub fn default_value(&self) -> Option<PropertyValue> {
        match self.default.as_ref()? {
            PropertyDefault::Fixed(value) => Some(value.clone()),
            PropertyDefault::With(f) => {
                let value = f();
                let ok = self.kind.accepts(&value);
                debug_assert!(
                    ok,
                    "o `.default_with` de \"{}\" não combina com o tipo da propriedade",
                    self.name
                );
                ok.then_some(value)
            }
        }
    }

    /// O valor atual, lido de forma rastreada: o signal ligado agora e o valor dele.
    pub fn value(&self) -> PropertyValue {
        match &self.source {
            Source::Text(prop) => PropertyValue::Text(prop.get().get()),
            Source::Number(prop) => PropertyValue::Number(prop.get().get()),
            Source::Bool(prop) => PropertyValue::Bool(prop.get().get()),
        }
    }

    /// O valor atual, sem registrar dependência.
    #[cfg(test)]
    pub fn value_untracked(&self) -> PropertyValue {
        use tomni_ui_reactive::GetUntracked;
        untrack(|| match &self.source {
            Source::Text(prop) => PropertyValue::Text(prop.get().get_untracked()),
            Source::Number(prop) => PropertyValue::Number(prop.get().get_untracked()),
            Source::Bool(prop) => PropertyValue::Bool(prop.get().get_untracked()),
        })
    }

    /// Se o valor difere do padrão (sem `.default`, nunca difere). Lê o `.default_with` de
    /// forma rastreada.
    pub fn changed(&self, value: &PropertyValue) -> bool {
        self.default_value().is_some_and(|d| d != *value)
    }

    /// O valor como a linha o mostra em repouso: a senha mascarada, o número formatado como o
    /// `NumberBox`, a `.check` sem texto.
    pub fn shown(&self, value: &PropertyValue) -> String {
        match (&self.kind, value) {
            (Kind::Password, PropertyValue::Text(text)) => {
                PASSWORD_CHAR.to_string().repeat(text.chars().count())
            }
            (_, PropertyValue::Text(text)) => text.clone(),
            (_, PropertyValue::Number(n)) => format_display(*n),
            (_, PropertyValue::Bool(_)) => String::new(),
        }
    }

    /// O valor para as ferramentas: como [`Property::shown`], e a `.check` como `true` ou
    /// `false`.
    pub fn described(&self, value: &PropertyValue) -> String {
        match value {
            PropertyValue::Bool(on) => on.to_string(),
            _ => self.shown(value),
        }
    }

    /// Se a linha escolhida troca o valor pelo editor: só a `.check` não (a caixa fica sempre
    /// à vista) e a só de leitura.
    pub fn editable(&self) -> bool {
        !self.read_only && !matches!(self.kind, Kind::Check)
    }

    /// Se está habilitada agora (sem registrar dependência).
    pub fn enabled_untracked(&self) -> bool {
        !untrack(|| self.disabled.get())
    }
}

/// Uma categoria.
pub(crate) struct Category {
    pub name: String,
    pub collapsed: bool,
    /// Se a categoria inteira (cabeçalho e linhas) não aparece.
    pub hidden: Prop<bool>,
}

/// Se a propriedade `i` não aparece: por ela mesma ou pela categoria dela. Lê as condições de
/// forma rastreada.
pub(crate) fn property_hidden(properties: &[Property], categories: &[Category], i: usize) -> bool {
    let property = &properties[i];
    property.hidden.get()
        || property
            .category
            .and_then(|c| categories.get(c))
            .is_some_and(|category| category.hidden.get())
}

/// Se a categoria `c` não aparece: escondida, ou sem nenhuma propriedade visível. Lê as
/// condições de forma rastreada.
pub(crate) fn category_hidden(properties: &[Property], categories: &[Category], c: usize) -> bool {
    categories[c].hidden.get()
        || !(0..properties.len()).any(|i| {
            properties[i].category == Some(c) && !property_hidden(properties, categories, i)
        })
}

/// Uma linha da grade: o cabeçalho de uma categoria ou uma propriedade, pelo índice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RowKey {
    Category(usize),
    Property(usize),
}

/// As linhas visíveis, em ordem.
///
/// - Por categoria: a ordem da declaração. As propriedades de antes da primeira categoria
///   ficam no topo, sem cabeçalho; as de uma categoria fechada somem.
/// - Alfabética: todas as propriedades, sem cabeçalhos, ordenadas sem acento nem maiúscula.
///   As categorias fechadas não contam.
/// - As propriedades escondidas e as de categoria escondida não aparecem em nenhuma ordem, e a
///   categoria sem nenhuma propriedade visível perde o cabeçalho. A categoria só fechada (com
///   linhas visíveis) guarda o cabeçalho.
pub(crate) fn visible(
    properties: &[Property],
    categories: &[Category],
    collapsed: &BTreeSet<String>,
    alphabetical: bool,
) -> Vec<RowKey> {
    if alphabetical {
        let mut order: Vec<usize> = (0..properties.len())
            .filter(|&i| !property_hidden(properties, categories, i))
            .collect();
        order.sort_by_cached_key(|&i| fold(&properties[i].name));
        return order.into_iter().map(RowKey::Property).collect();
    }
    let mut rows = Vec::new();
    for (i, property) in properties.iter().enumerate() {
        if property.category.is_none() && !property_hidden(properties, categories, i) {
            rows.push(RowKey::Property(i));
        }
    }
    for (c, category) in categories.iter().enumerate() {
        if category_hidden(properties, categories, c) {
            continue;
        }
        rows.push(RowKey::Category(c));
        if collapsed.contains(&category.name) {
            continue;
        }
        for (i, property) in properties.iter().enumerate() {
            if property.category == Some(c) && !property_hidden(properties, categories, i) {
                rows.push(RowKey::Property(i));
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use tomni_ui_reactive::prelude::*;

    use super::*;

    fn texto(nome: &str, categoria: Option<usize>) -> Property {
        Property::new(
            nome.into(),
            categoria,
            Kind::Text,
            Source::Text(Prop::Value(signal(String::new()))),
        )
    }

    fn categoria(nome: &str) -> Category {
        Category {
            name: nome.into(),
            collapsed: false,
            hidden: Prop::Value(false),
        }
    }

    fn nomes(
        propriedades: &[Property],
        categorias: &[Category],
        fechadas: &[&str],
        alfabetica: bool,
    ) -> Vec<String> {
        let fechadas = fechadas.iter().map(ToString::to_string).collect();
        visible(propriedades, categorias, &fechadas, alfabetica)
            .into_iter()
            .map(|linha| match linha {
                RowKey::Category(c) => format!("[{}]", categorias[c].name),
                RowKey::Property(p) => propriedades[p].name.clone(),
            })
            .collect()
    }

    #[test]
    fn linhas_por_categoria_na_ordem_da_declaracao() {
        let scope = Scope::root();
        scope.run(|| {
            let categorias = [categoria("Conexão"), categoria("Terminal")];
            let propriedades = [
                texto("Solta", None),
                texto("Host", Some(0)),
                texto("Porta", Some(0)),
                texto("Bipe", Some(1)),
            ];
            assert_eq!(
                nomes(&propriedades, &categorias, &[], false),
                ["Solta", "[Conexão]", "Host", "Porta", "[Terminal]", "Bipe"]
            );
        });
        scope.clear();
    }

    #[test]
    fn categoria_fechada_guarda_o_cabecalho_e_esconde_as_propriedades() {
        let scope = Scope::root();
        scope.run(|| {
            let categorias = [categoria("Conexão"), categoria("Terminal")];
            let propriedades = [texto("Host", Some(0)), texto("Bipe", Some(1))];
            assert_eq!(
                nomes(&propriedades, &categorias, &["Conexão"], false),
                ["[Conexão]", "[Terminal]", "Bipe"]
            );
            assert_eq!(
                nomes(&propriedades, &categorias, &["Conexão"], true),
                ["Bipe", "Host"],
                "na alfabética as fechadas não escondem nada"
            );
        });
        scope.clear();
    }

    #[test]
    fn alfabetica_ignora_acento_e_maiuscula() {
        let scope = Scope::root();
        scope.run(|| {
            let categorias = [categoria("A")];
            let propriedades = [
                texto("zebra", Some(0)),
                texto("Ágil", Some(0)),
                texto("Agenda", None),
                texto("barco", Some(0)),
            ];
            assert_eq!(
                nomes(&propriedades, &categorias, &[], true),
                ["Agenda", "Ágil", "barco", "zebra"]
            );
        });
        scope.clear();
    }

    #[test]
    fn valor_mostrado_e_descrito() {
        let scope = Scope::root();
        scope.run(|| {
            let senha = Property::new(
                "Senha".into(),
                None,
                Kind::Password,
                Source::Text(Prop::Value(signal("abç".to_owned()))),
            );
            let valor = senha.value_untracked();
            assert_eq!(senha.shown(&valor), "•••");
            assert_eq!(senha.described(&valor), "•••");

            let porta = Property::new(
                "Porta".into(),
                None,
                Kind::Number {
                    min: 1.0,
                    max: 65535.0,
                },
                Source::Number(Prop::Value(signal(22.5))),
            );
            assert_eq!(porta.shown(&porta.value_untracked()), "22,5");

            let bipe = Property::new(
                "Bipe".into(),
                None,
                Kind::Check,
                Source::Bool(Prop::Value(signal(true))),
            );
            let valor = bipe.value_untracked();
            assert_eq!(bipe.shown(&valor), "");
            assert_eq!(bipe.described(&valor), "true");
        });
        scope.clear();
    }

    #[test]
    fn changed_acompanha_o_default_nos_tres_tipos() {
        let scope = Scope::root();
        scope.run(|| {
            let texto_sinal = signal("a".to_owned());
            let mut host = Property::new(
                "Host".into(),
                None,
                Kind::Text,
                Source::Text(Prop::Value(texto_sinal)),
            );
            assert!(!host.changed(&host.value_untracked()), "sem default");
            host.set_default("a".into());
            assert!(!host.changed(&host.value_untracked()));
            texto_sinal.set("b".into());
            assert!(host.changed(&host.value_untracked()));
            texto_sinal.set("a".into());
            assert!(!host.changed(&host.value_untracked()));

            let numero = signal(22.0);
            let mut porta = Property::new(
                "Porta".into(),
                None,
                Kind::Number {
                    min: 1.0,
                    max: 65535.0,
                },
                Source::Number(Prop::Value(numero)),
            );
            porta.set_default(22.into());
            assert!(!porta.changed(&porta.value_untracked()));
            numero.set(2222.0);
            assert!(porta.changed(&porta.value_untracked()));

            let marca = signal(true);
            let mut bipe = Property::new(
                "Bipe".into(),
                None,
                Kind::Check,
                Source::Bool(Prop::Value(marca)),
            );
            bipe.set_default(true.into());
            assert!(!bipe.changed(&bipe.value_untracked()));
            marca.set(false);
            assert!(bipe.changed(&bipe.value_untracked()));
        });
        scope.clear();
    }

    fn escondida(mut propriedade: Property, condicao: bool) -> Property {
        propriedade.hidden = Prop::Value(condicao);
        propriedade
    }

    fn categoria_escondida(nome: &str, condicao: bool) -> Category {
        Category {
            name: nome.into(),
            collapsed: false,
            hidden: Prop::Value(condicao),
        }
    }

    #[test]
    fn propriedade_escondida_sai_por_categoria_e_na_alfabetica() {
        let scope = Scope::root();
        scope.run(|| {
            let categorias = [categoria("Conexão")];
            let propriedades = [
                escondida(texto("Solta", None), true),
                texto("Visível", None),
                texto("Host", Some(0)),
                escondida(texto("Porta", Some(0)), true),
            ];
            assert_eq!(
                nomes(&propriedades, &categorias, &[], false),
                ["Visível", "[Conexão]", "Host"]
            );
            assert_eq!(
                nomes(&propriedades, &categorias, &[], true),
                ["Host", "Visível"]
            );
        });
        scope.clear();
    }

    #[test]
    fn categoria_sem_linha_visivel_perde_o_cabecalho() {
        let scope = Scope::root();
        scope.run(|| {
            let categorias = [categoria("A"), categoria("B")];
            let todas = [
                escondida(texto("a1", Some(0)), true),
                escondida(texto("a2", Some(0)), true),
                texto("b1", Some(1)),
            ];
            assert_eq!(nomes(&todas, &categorias, &[], false), ["[B]", "b1"]);
            let uma = [
                escondida(texto("a1", Some(0)), true),
                texto("a2", Some(0)),
                texto("b1", Some(1)),
            ];
            assert_eq!(
                nomes(&uma, &categorias, &[], false),
                ["[A]", "a2", "[B]", "b1"]
            );
        });
        scope.clear();
    }

    #[test]
    fn categoria_escondida_some_com_as_linhas_em_qualquer_ordem() {
        let scope = Scope::root();
        scope.run(|| {
            let categorias = [categoria_escondida("A", true), categoria("B")];
            let propriedades = [
                texto("a1", Some(0)),
                texto("b1", Some(1)),
                texto("solta", None),
            ];
            assert_eq!(
                nomes(&propriedades, &categorias, &[], false),
                ["solta", "[B]", "b1"]
            );
            assert_eq!(
                nomes(&propriedades, &categorias, &[], true),
                ["b1", "solta"]
            );
        });
        scope.clear();
    }

    #[test]
    fn categoria_fechada_com_linhas_visiveis_guarda_o_cabecalho() {
        let scope = Scope::root();
        scope.run(|| {
            let categorias = [categoria("A")];
            let propriedades = [escondida(texto("a1", Some(0)), true), texto("a2", Some(0))];
            assert_eq!(nomes(&propriedades, &categorias, &["A"], false), ["[A]"]);
        });
        scope.clear();
    }

    #[test]
    fn changed_acompanha_o_default_calculado() {
        let scope = Scope::root();
        scope.run(|| {
            let tipo = signal(String::from("ssh"));
            let porta = signal(22.0);
            let mut propriedade = Property::new(
                "Porta".into(),
                None,
                Kind::Number {
                    min: 1.0,
                    max: 65535.0,
                },
                Source::Number(Prop::Value(porta)),
            );
            propriedade.set_default_with(Rc::new(move || {
                PropertyValue::Number(if tipo.get_untracked() == "telnet" {
                    23.0
                } else {
                    22.0
                })
            }));
            assert!(!propriedade.changed(&propriedade.value_untracked()));
            tipo.set("telnet".into());
            assert!(propriedade.changed(&propriedade.value_untracked()));
            porta.set(23.0);
            assert!(!propriedade.changed(&propriedade.value_untracked()));
        });
        scope.clear();
    }

    #[test]
    #[should_panic(expected = "não combina com o tipo")]
    fn default_de_outro_tipo_avisa_em_debug() {
        let scope = Scope::root();
        scope.run(|| {
            let mut porta = Property::new(
                "Porta".into(),
                None,
                Kind::Number { min: 0.0, max: 1.0 },
                Source::Number(Prop::Value(signal(0.0))),
            );
            porta.set_default("22".into());
        });
        scope.clear();
    }
}
