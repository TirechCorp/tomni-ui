//! Arrastar e soltar dentro da janela.

/// Quanto o ponteiro anda com o botão pressionado antes de o arrasto começar: o limiar de
/// arrasto do Windows (`SM_CXDRAG`, 4 px).
pub const DRAG_THRESHOLD: f64 = 4.0;

/// O que está sendo arrastado ([`Widget::drag_data`](crate::Widget::drag_data)).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DragData {
    /// Tipo do item: o alvo aceita ou não por ele.
    pub kind: String,
    /// Chave do item.
    pub key: String,
    /// Texto da etiqueta que segue o ponteiro.
    pub label: String,
}

/// Onde o item arrastado vai cair em relação a um item da lista: a linha antes ou depois
/// dele.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InsertMark {
    /// Antes: à esquerda numa fileira, acima numa coluna.
    Before,
    /// Depois: à direita numa fileira, abaixo numa coluna.
    After,
}
