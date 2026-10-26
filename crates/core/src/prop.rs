//! Propriedades que aceitam um valor fixo, uma closure ou um signal.

use std::fmt;
use std::rc::Rc;

use tomni_ui_reactive::{Get, Memo, Signal};

/// Valor de uma propriedade de widget: fixo ou reativo.
///
/// Lida dentro de um efeito (o paint de um widget, por exemplo), uma propriedade reativa
/// registra a dependência como qualquer signal.
pub enum Prop<T> {
    /// Valor fixo.
    Value(T),
    /// Recalculado a cada leitura.
    Dynamic(Rc<dyn Fn() -> T>),
}

impl<T: Clone> Prop<T> {
    /// Valor atual.
    pub fn get(&self) -> T {
        match self {
            Self::Value(value) => value.clone(),
            Self::Dynamic(f) => f(),
        }
    }
}

impl<T: Clone> Clone for Prop<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Value(value) => Self::Value(value.clone()),
            Self::Dynamic(f) => Self::Dynamic(Rc::clone(f)),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Prop<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => f.debug_tuple("Value").field(value).finish(),
            Self::Dynamic(_) => f.write_str("Dynamic(..)"),
        }
    }
}

impl<T: Default> Default for Prop<T> {
    fn default() -> Self {
        Self::Value(T::default())
    }
}

/// Conversão para [`Prop`], usada pelos builders: `.disabled(true)`,
/// `.disabled(move || !valido.get())` ou `.disabled(sinal)`.
///
/// `Into<Prop<T>>` não serve: `From<T>` e `From<F: Fn() -> T>` se sobrepõem para o compilador
/// (E0119). O parâmetro `Marker` separa os casos e é inferido a partir do `T` que o builder
/// pede; numa função genérica em `T`, uma closure é ambígua e precisa do tipo explícito.
#[diagnostic::on_unimplemented(
    message = "`{Self}` não serve como propriedade do tipo `{T}`",
    label = "esperava `{T}`, uma closure que devolve `{T}` ou um signal de `{T}`",
    note = "aceita um valor fixo (`true`, `\"Entrar\"`), uma closure (`move || !valido.get()`) ou um \
            `Signal`/`Memo` do mesmo tipo",
    note = "texto montado na hora vira closure: `move || format!(\"Cliques: {{}}\", n.get())`"
)]
pub trait IntoProp<T, Marker> {
    /// Converte para [`Prop`].
    fn into_prop(self) -> Prop<T>;
}

/// Marcadores de [`IntoProp`]. Não aparecem no código dos apps.
pub mod marker {
    /// Valor fixo.
    #[derive(Debug)]
    pub enum Value {}
    /// `&str` para `Prop<String>`.
    #[derive(Debug)]
    pub enum Str {}
    /// Closure.
    #[derive(Debug)]
    pub enum Closure {}
    /// [`Signal`](tomni_ui_reactive::Signal) ou [`Memo`](tomni_ui_reactive::Memo).
    #[derive(Debug)]
    pub enum Reactive {}
}

impl<T> IntoProp<T, marker::Value> for T {
    fn into_prop(self) -> Prop<T> {
        Prop::Value(self)
    }
}

impl IntoProp<String, marker::Str> for &str {
    fn into_prop(self) -> Prop<String> {
        Prop::Value(self.to_owned())
    }
}

impl<T, F: Fn() -> T + 'static> IntoProp<T, marker::Closure> for F {
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Rc::new(self))
    }
}

impl<T: Clone + Send + Sync + 'static> IntoProp<T, marker::Reactive> for Signal<T> {
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Rc::new(move || self.get()))
    }
}

impl<T: Clone + PartialEq + Send + Sync + 'static> IntoProp<T, marker::Reactive> for Memo<T> {
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Rc::new(move || self.get()))
    }
}

#[cfg(test)]
mod tests {
    use tomni_ui_reactive::prelude::*;

    use super::*;

    fn prop<T, M>(value: impl IntoProp<T, M>) -> Prop<T> {
        value.into_prop()
    }

    #[test]
    fn aceita_valor_str_closure_e_signal() {
        let scope = Scope::root();
        scope.run(|| {
            assert!(prop::<bool, _>(true).get());
            assert_eq!(prop::<String, _>("Entrar").get(), "Entrar");

            let n = signal(1);
            let dobro = memo(move || n.get() * 2);
            let closure = prop(move || n.get() + 10);
            let sinal = prop(n);
            let derivado = prop(dobro);
            n.set(5);
            assert_eq!((closure.get(), sinal.get(), derivado.get()), (15, 5, 10));
        });
        scope.clear();
    }

    #[test]
    fn leitura_dentro_de_efeito_rastreia() {
        let scope = Scope::root();
        let lidos = Rc::new(std::cell::RefCell::new(Vec::<bool>::new()));
        let n = scope.run(|| {
            let n = signal(false);
            let disabled = prop::<bool, _>(move || !n.get());
            let lidos = Rc::clone(&lidos);
            effect(move || lidos.borrow_mut().push(disabled.get()));
            n
        });
        run_effects();
        n.set(true);
        run_effects();
        assert_eq!(*lidos.borrow(), [true, false]);
        scope.clear();
    }
}
