//! Copiar texto para a área de transferência de qualquer lugar do app, inclusive do `on_click`
//! de um botão, que não tem acesso ao [`Clipboard`].
//!
//! ```ignore
//! Button::new("Copiar senha").on_click(move || clipboard::write(senha.get_untracked()))
//! ```
//!
//! O texto espera numa fila da thread e o `Ui` o entrega ao clipboard da janela no próximo
//! quadro (assim como o pedido de [`read`] e de [`clear_if`]). Nos testes, `app.frame()` entrega e `app.clipboard().read()` devolve o texto.

use std::cell::RefCell;

use crate::cx::Clipboard;
pub use crate::cx::ClipboardImage;

/// Pedido que mexe direto na área de transferência da plataforma.
type Acesso = Box<dyn FnOnce(&mut dyn Clipboard)>;

enum Op {
    Write(String),
    Read(Box<dyn FnOnce(Option<String>)>),
    With(Acesso),
}

thread_local! {
    static FILA: RefCell<Vec<Op>> = const { RefCell::new(Vec::new()) };
}

fn push(op: Op) {
    FILA.with(|fila| fila.borrow_mut().push(op));
    crate::wake();
}

/// Copia `text` para a área de transferência, no próximo quadro.
pub fn write(text: impl Into<String>) {
    push(Op::Write(text.into()));
}

/// Lê o texto da área de transferência no próximo quadro e chama `f` com ele (`None` se não
/// há texto). Nunca roda dentro da chamada, como as respostas dos diálogos.
pub fn read(f: impl FnOnce(Option<String>) + 'static) {
    push(Op::Read(Box::new(f)));
}

/// Chama `f` com a área de transferência inteira (texto, arquivos e imagem) no próximo quadro.
/// Nunca roda dentro da chamada. É o que o "Colar" do menu do `TextEditor` usa para ler os
/// arquivos, o texto e a imagem na ordem certa.
pub fn with(f: impl FnOnce(&mut dyn Clipboard) + 'static) {
    push(Op::With(Box::new(f)));
}

/// Apaga a área de transferência no próximo quadro, **se** ela ainda tem exatamente `text`: é
/// o "limpar depois de copiar uma senha" sem apagar o que a pessoa copiou depois.
pub fn clear_if(text: impl Into<String>) {
    let text = text.into();
    read(move |atual| {
        if atual.as_deref() == Some(text.as_str()) {
            write("");
        }
    });
}

/// Executa o que [`write`], [`read`] e [`clear_if`] guardaram, na ordem, em `clipboard`. O
/// `Ui` chama a cada quadro. O que um callback pedir de novo roda na mesma chamada.
#[doc(hidden)]
pub fn flush(clipboard: &mut dyn Clipboard) {
    loop {
        let ops = FILA.with(|fila| std::mem::take(&mut *fila.borrow_mut()));
        if ops.is_empty() {
            return;
        }
        for op in ops {
            match op {
                Op::Write(text) => clipboard.write(text),
                Op::Read(f) => f(clipboard.read()),
                Op::With(f) => f(clipboard),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[derive(Default)]
    struct Falso(Option<String>);

    impl Clipboard for Falso {
        fn read(&mut self) -> Option<String> {
            self.0.clone()
        }

        fn write(&mut self, text: String) {
            self.0 = Some(text);
        }
    }

    #[test]
    fn a_fila_guarda_na_ordem_e_esvazia() {
        let mut falso = Falso::default();
        flush(&mut falso);
        assert_eq!(falso.0, None);
        write("um");
        write(String::from("dois"));
        flush(&mut falso);
        assert_eq!(falso.0.as_deref(), Some("dois"));
    }

    #[test]
    fn read_entrega_o_texto_no_flush_e_nunca_na_hora() {
        let mut falso = Falso(Some("copiado".into()));
        let lido = Rc::new(Cell::new(false));
        let l = Rc::clone(&lido);
        read(move |texto| {
            assert_eq!(texto.as_deref(), Some("copiado"));
            l.set(true);
        });
        assert!(!lido.get());
        flush(&mut falso);
        assert!(lido.get());
    }

    #[test]
    fn with_entrega_a_area_de_transferencia_no_flush() {
        let mut falso = Falso(Some("copiado".into()));
        let lido = Rc::new(Cell::new(false));
        let l = Rc::clone(&lido);
        with(move |clipboard| {
            assert_eq!(clipboard.read().as_deref(), Some("copiado"));
            clipboard.write("novo".into());
            l.set(true);
        });
        assert!(!lido.get());
        flush(&mut falso);
        assert!(lido.get());
        assert_eq!(falso.0.as_deref(), Some("novo"));
    }

    #[test]
    fn clear_if_so_apaga_o_que_o_app_copiou() {
        let mut falso = Falso(Some("senha".into()));
        clear_if("senha");
        flush(&mut falso);
        assert_eq!(falso.0.as_deref(), Some(""));

        let mut falso = Falso(Some("outra coisa".into()));
        clear_if("senha");
        flush(&mut falso);
        assert_eq!(falso.0.as_deref(), Some("outra coisa"));
    }
}
