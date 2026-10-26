//! `Show` e `For`: fragmentos que trocam os filhos quando os dados mudam.

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::rc::Rc;

use crate::prop::{IntoProp, Prop};
use crate::widget::{Common, Look, Slot, View, Widget};

type MakeView = Rc<dyn Fn() -> Box<dyn Widget>>;

/// Mostra uma view enquanto a condição vale e, com [`Show::otherwise`], outra quando não vale.
///
/// ```
/// use tomni_ui_core::{Button, Show, Text};
/// use tomni_ui_reactive::prelude::*;
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let logado = signal(false);
/// let conta = Show::when(logado, || Button::new("Sair").id("sair"))
///     .otherwise(|| Text::new("Entre para continuar").id("aviso"));
/// # });
/// # scope.clear();
/// ```
///
/// Quando a condição muda, a view que sai é descartada (widgets, estado, efeitos e signals
/// criados dentro dela) e a que entra é criada do zero. Para esconder um widget mantendo o
/// estado (o texto digitado num `Edit`, por exemplo), use `.visible()`.
///
/// O `Show` não ocupa lugar: a view entra direto no contêiner de cima, como se estivesse no
/// lugar dele.
pub struct Show {
    common: Common,
    when: Prop<bool>,
    view: MakeView,
    otherwise: Option<MakeView>,
    shown: Option<bool>,
}

impl Show {
    /// Mostra `view` enquanto `when` (valor fixo, closure ou signal) for `true`.
    pub fn when<M, V: View>(when: impl IntoProp<bool, M>, view: impl Fn() -> V + 'static) -> Self {
        Self {
            common: Common::default(),
            when: when.into_prop(),
            view: Rc::new(move || Box::new(view())),
            otherwise: None,
            shown: None,
        }
    }

    /// View mostrada quando a condição é `false`.
    pub fn otherwise<V: View>(mut self, view: impl Fn() -> V + 'static) -> Self {
        self.otherwise = Some(Rc::new(move || Box::new(view())));
        self
    }

    /// Id estável, usado em testes, no MCP e no lint.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.common.id = Some(id.into());
        self
    }
}

impl Widget for Show {
    fn kind(&self) -> &'static str {
        "show"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Fragment
    }

    fn fragment(&self) -> bool {
        true
    }

    fn track_children(&self) {
        let _ = self.when.get();
    }

    fn reconcile(&mut self) -> Option<Vec<Slot>> {
        let when = self.when.get();
        if self.shown == Some(when) {
            return None;
        }
        self.shown = Some(when);
        let view = if when {
            Some(&self.view)
        } else {
            self.otherwise.as_ref()
        };
        Some(
            view.map(|view| {
                let view = Rc::clone(view);
                Slot::New(Box::new(move || view()))
            })
            .into_iter()
            .collect(),
        )
    }
}

impl fmt::Debug for Show {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Show")
            .field("common", &self.common)
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

/// Uma view para cada item de uma lista, identificada por uma chave.
///
/// ```
/// use tomni_ui_core::{CheckBox, Column, For};
/// use tomni_ui_reactive::prelude::*;
///
/// #[derive(Clone)]
/// struct Tarefa {
///     id: u32,
///     texto: String,
///     feita: Signal<bool>,
/// }
///
/// # let scope = Scope::root();
/// # scope.run(|| {
/// let tarefas = signal(Vec::<Tarefa>::new());
/// let lista = Column::new().gap(4).child(For::each(
///     tarefas,
///     |t| t.id,
///     |t| CheckBox::bind(t.feita, t.texto).id(format!("tarefa-{}", t.id)),
/// ));
/// # });
/// # scope.clear();
/// ```
///
/// Quando a lista muda, o `For` compara as chaves: itens novos ganham uma view nova, itens
/// que saíram têm a view descartada (com o estado, os efeitos e os signals criados nela) e os
/// que ficaram mantêm a view e o estado (texto, foco, hover), mesmo mudando de posição.
///
/// A view de cada item é criada uma vez, com o valor do item naquele momento: mudar um item
/// sem mudar a chave não cria a view de novo. O que muda num item fica num signal dentro dele
/// (o `feita` acima) ou é lido por uma closure que procura o item pela chave.
///
/// As chaves precisam ser únicas; um item com chave repetida ganha uma view nova a cada
/// mudança da lista.
///
/// O `For` não ocupa lugar: as views entram direto no contêiner de cima, com o `gap` dele.
pub struct For<T, K> {
    common: Common,
    items: Prop<Vec<T>>,
    key: Rc<dyn Fn(&T) -> K>,
    view: Rc<dyn Fn(T) -> Box<dyn Widget>>,
    keys: Option<Vec<K>>,
}

impl<T: Clone + 'static, K: Eq + Hash + Clone + 'static> For<T, K> {
    /// Uma view `view(item)` para cada item de `items` (valor fixo, closure ou signal), com a
    /// chave `key(&item)`.
    pub fn each<M, V: View>(
        items: impl IntoProp<Vec<T>, M>,
        key: impl Fn(&T) -> K + 'static,
        view: impl Fn(T) -> V + 'static,
    ) -> Self {
        Self {
            common: Common::default(),
            items: items.into_prop(),
            key: Rc::new(key),
            view: Rc::new(move |item| Box::new(view(item))),
            keys: None,
        }
    }

    /// Id estável, usado em testes, no MCP e no lint.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.common.id = Some(id.into());
        self
    }
}

impl<T: Clone + 'static, K: Eq + Hash + Clone + 'static> Widget for For<T, K> {
    fn kind(&self) -> &'static str {
        "for"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Fragment
    }

    fn fragment(&self) -> bool {
        true
    }

    fn track_children(&self) {
        let _ = self.items.get();
    }

    fn reconcile(&mut self) -> Option<Vec<Slot>> {
        let items = self.items.get();
        let keys: Vec<K> = items.iter().map(|item| (self.key)(item)).collect();
        if self.keys.as_ref() == Some(&keys) {
            return None;
        }
        let mut old: HashMap<K, usize> = self
            .keys
            .iter()
            .flatten()
            .enumerate()
            .map(|(i, key)| (key.clone(), i))
            .collect();
        let slots = items
            .into_iter()
            .zip(&keys)
            .map(|(item, key)| match old.remove(key) {
                Some(i) => Slot::Keep(i),
                None => {
                    let view = Rc::clone(&self.view);
                    Slot::New(Box::new(move || view(item)))
                }
            })
            .collect();
        self.keys = Some(keys);
        Some(slots)
    }
}

impl<T, K> fmt::Debug for For<T, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("For")
            .field("common", &self.common)
            .field("items", &self.keys.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Text;

    fn kinds(slots: &[Slot]) -> String {
        slots
            .iter()
            .map(|slot| match slot {
                Slot::Keep(i) => i.to_string(),
                Slot::New(_) => "+".into(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn for_compara_as_chaves() {
        let items = Rc::new(std::cell::RefCell::new(vec![1, 2, 3]));
        let lidos = Rc::clone(&items);
        let mut lista = For::each(
            move || lidos.borrow().clone(),
            |n| *n,
            |n| Text::new(n.to_string()),
        );
        assert_eq!(kinds(&lista.reconcile().unwrap()), "+ + +");
        assert!(lista.reconcile().is_none(), "nada mudou");
        *items.borrow_mut() = vec![3, 1, 4];
        assert_eq!(kinds(&lista.reconcile().unwrap()), "2 0 +");
        *items.borrow_mut() = vec![4, 4];
        assert_eq!(
            kinds(&lista.reconcile().unwrap()),
            "2 +",
            "chave repetida é nova"
        );
    }

    #[test]
    fn show_troca_a_view() {
        let quando = Rc::new(std::cell::Cell::new(true));
        let lido = Rc::clone(&quando);
        let mut show = Show::when(move || lido.get(), || Text::new("sim"));
        assert_eq!(kinds(&show.reconcile().unwrap()), "+");
        assert!(show.reconcile().is_none());
        quando.set(false);
        assert_eq!(
            kinds(&show.reconcile().unwrap()),
            "",
            "sem otherwise, fica vazio"
        );
        let mut show = Show::when(false, || Text::new("sim")).otherwise(|| Text::new("não"));
        assert_eq!(kinds(&show.reconcile().unwrap()), "+");
    }
}
