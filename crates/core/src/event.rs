//! Eventos de entrada, já direcionados a um widget.
//!
//! A plataforma traduz o que vem do sistema e a árvore faz o hit test e a captura: o widget
//! recebe um evento já sabendo se o ponteiro está sobre ele.

/// Dispositivo do ponteiro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    /// Mouse ou touchpad.
    Mouse,
    /// Dedo. Não existe hover no toque.
    Touch,
    /// Caneta.
    Pen,
}

impl PointerKind {
    /// Se o dispositivo mostra hover quando está sobre um widget sem pressionar.
    pub fn hovers(self) -> bool {
        self != Self::Touch
    }
}

/// Tecla com significado para os widgets do piloto.
///
/// O texto digitado não vem daqui: chega como [`Event::Text`], já com o layout do teclado e as
/// teclas mortas aplicados. `Key` serve para navegação e atalhos.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// Enter.
    Enter,
    /// Barra de espaço.
    Space,
    /// Tab. A árvore usa para mover o foco.
    Tab,
    /// Esc.
    Escape,
    /// Apaga para trás.
    Backspace,
    /// Apaga para a frente.
    Delete,
    /// Seta para a esquerda.
    Left,
    /// Seta para a direita.
    Right,
    /// Seta para cima.
    Up,
    /// Seta para baixo.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up: rola uma página para cima.
    PageUp,
    /// Page Down: rola uma página para baixo.
    PageDown,
    /// Insert.
    Insert,
    /// Tecla de função, de F1 a F24. O F6 tira o foco do `Terminal`, que fica com o Tab.
    F(u8),
    /// Alt, sozinha. Pressionada e solta sem outra tecla no meio, põe o foco na `MenuBar`.
    Alt,
    /// Tecla Menu (a de aplicativo, ao lado do Ctrl da direita): abre o menu de contexto do
    /// widget com foco, como o Shift+F10.
    ContextMenu,
    /// Tecla de letra ou número, em minúscula, para atalhos (`Ctrl+A` é `Key::Char('a')` com
    /// [`Modifiers::COMMAND`]).
    Char(char),
    /// Qualquer outra tecla.
    Other,
}

/// Teclas modificadoras pressionadas. A plataforma avisa quando mudam, e a árvore repassa o
/// estado atual ao widget junto com cada evento.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// Shift.
    pub shift: bool,
    /// Ctrl.
    pub ctrl: bool,
    /// Alt (Option no macOS).
    pub alt: bool,
    /// Tecla do sistema: Cmd no macOS, Windows no Windows.
    pub meta: bool,
}

impl Modifiers {
    /// Nenhuma.
    pub const NONE: Self = Self {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    };

    /// Só Shift.
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };

    /// Só Ctrl, em qualquer sistema: Ctrl+Tab troca de aba no `TabView`.
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };

    /// Só Alt (Option no macOS): Alt+↓ abre o `ComboBox`.
    pub const ALT: Self = Self {
        alt: true,
        ..Self::NONE
    };

    /// A tecla dos atalhos de edição: Cmd no macOS e no iOS, Ctrl no resto.
    pub const COMMAND: Self = if cfg!(any(target_os = "macos", target_os = "ios")) {
        Self {
            meta: true,
            ..Self::NONE
        }
    } else {
        Self {
            ctrl: true,
            ..Self::NONE
        }
    };

    /// Se a tecla dos atalhos de edição ([`Modifiers::COMMAND`]) está pressionada.
    pub const fn command(self) -> bool {
        if cfg!(any(target_os = "macos", target_os = "ios")) {
            self.meta
        } else {
            self.ctrl
        }
    }

    /// Combinação das duas.
    pub const fn with(self, other: Self) -> Self {
        Self {
            shift: self.shift || other.shift,
            ctrl: self.ctrl || other.ctrl,
            alt: self.alt || other.alt,
            meta: self.meta || other.meta,
        }
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.with(other)
    }
}

/// Composição do IME (japonês, chinês, coreano e acentos em alguns sistemas).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ime {
    /// Texto ainda em composição, mostrado sublinhado no cursor. `cursor` é o trecho
    /// destacado pelo IME, em bytes de `text`. Texto vazio encerra a composição sem inserir
    /// nada.
    Preedit {
        /// Texto em composição.
        text: String,
        /// Cursor ou trecho selecionado dentro de `text`.
        cursor: Option<(usize, usize)>,
    },
    /// Texto confirmado, que entra no lugar da composição.
    Commit(String),
}

/// Como o widget recebeu o foco. O anel de foco só aparece com [`FocusSource::Keyboard`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusSource {
    /// Tab, Shift+Tab ou foco programático vindo do teclado.
    Keyboard,
    /// Clique ou toque.
    Pointer,
}

/// Evento entregue a um widget.
///
/// A posição do ponteiro e os modificadores vêm no [`EventCx`](crate::EventCx).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// O ponteiro se moveu; `inside` diz se está sobre o widget. Também é entregue ao widget
    /// que o ponteiro acabou de deixar, com `inside: false`.
    PointerMove {
        /// Dispositivo.
        kind: PointerKind,
        /// Se o ponteiro está sobre o widget.
        inside: bool,
    },
    /// Botão primário ou dedo pressionado sobre o widget. A árvore captura o ponteiro: o
    /// `PointerMove` e o `PointerUp` seguintes vão para este widget mesmo fora dele.
    PointerDown {
        /// Dispositivo.
        kind: PointerKind,
        /// 1 no clique simples, 2 no duplo, 3 no triplo. Quem conta é a plataforma, que sabe o
        /// intervalo de duplo clique do sistema.
        clicks: u8,
    },
    /// Botão secundário (o direito) pressionado sobre o widget. Não captura o ponteiro nem
    /// move o foco; o `Terminal` cola.
    SecondaryDown,
    /// O botão secundário (o direito) foi solto. Chega só ao widget que usou o
    /// [`Event::SecondaryDown`], esteja o ponteiro onde estiver (o `Terminal` que passa o mouse
    /// ao programa avisa que o botão subiu).
    SecondaryUp,
    /// O menu de contexto do widget (`.context_flyout()`) vai abrir, por qualquer caminho
    /// (botão direito, Shift+F10, tecla Menu, toque longo). Só o dono do menu recebe, antes de
    /// abrir. Devolver `true` pede o foco: se o widget aceita foco, ele o recebe (o item de uma
    /// lista que o clique direito seleciona, como no Explorador); `false` deixa o foco onde
    /// está.
    ContextOpening,
    /// Botão do meio pressionado e solto sobre o mesmo widget, como o clique do meio que fecha
    /// uma aba no WinUI. Não captura o ponteiro nem move o foco.
    MiddleClick,
    /// Botão do meio pressionado sobre o widget (só o widget sob o ponteiro recebe, sem subir
    /// aos ancestrais). Quem usa recebe depois o [`Event::MiddleUp`].
    MiddleDown,
    /// O botão do meio foi solto. Chega ao widget que recebeu o [`Event::MiddleDown`], esteja o
    /// ponteiro onde estiver; se ele usar, o [`Event::MiddleClick`] não sai.
    MiddleUp,
    /// Botão primário ou dedo solto.
    PointerUp {
        /// Dispositivo.
        kind: PointerKind,
        /// Se o ponteiro está sobre o widget.
        inside: bool,
    },
    /// O ponteiro saiu da janela ou o sistema cancelou o gesto.
    PointerCancel,
    /// Tecla pressionada, com o foco no widget. Repetições também chegam aqui.
    KeyDown(Key),
    /// Tecla solta, com o foco no widget.
    KeyUp(Key),
    /// Texto digitado, com o foco no widget.
    Text(String),
    /// Composição do IME, com o foco no widget.
    Ime(Ime),
    /// O widget recebeu o foco.
    FocusIn(FocusSource),
    /// O widget perdeu o foco.
    FocusOut,
    /// Roda do mouse com o ponteiro sobre o widget que tem o foco, antes de rolar a tela. O
    /// `NumberBox` usa para mudar o valor; quem não consome deixa a roda rolar.
    Wheel {
        /// Se a roda girou para cima (para longe de quem usa).
        up: bool,
    },
    /// Roda do mouse ou touchpad sobre um widget com
    /// [`Widget::wants_scroll`](crate::Widget::wants_scroll), em pixels arredondados: `y`
    /// positivo mostra o que está mais abaixo. Chega antes de o `ScrollViewer` rolar; quem
    /// consome fica com a rolagem.
    Scroll {
        /// Pixels na horizontal.
        x: i32,
        /// Pixels na vertical.
        y: i32,
    },
    /// Um item arrastado ([`Widget::drag_data`](crate::Widget::drag_data)) passa sobre o
    /// widget: vai ao widget sob o ponteiro e, se ele não usar, aos ancestrais. Quem consome
    /// aceita o item (pelo tipo) e passa a ser o alvo: mostra onde ele vai cair, com o ponteiro
    /// em [`EventCx::pointer`](crate::EventCx::pointer).
    DragOver(crate::DragData),
    /// O item arrastado saiu do alvo, ou o arrasto foi cancelado (Esc): o alvo apaga a marca.
    DragLeave,
    /// O item foi solto sobre o alvo, que faz a mudança e apaga a marca.
    Drop(crate::DragData),
    /// O item arrastado foi solto fora de todas as janelas do processo. Chega só ao widget de
    /// onde ele saiu. `at` é o ponto na tela, em pixels lógicos; `None` onde o sistema não
    /// informa a posição das janelas (o Wayland).
    DropOutside {
        /// O item.
        data: crate::DragData,
        /// Onde caiu, na tela.
        at: Option<(i32, i32)>,
    },
    /// Arquivos do sistema (o gerenciador de arquivos do usuário) foram soltos na janela: vai ao
    /// widget sob o ponteiro e, se ele não usar, aos ancestrais. O ponto do soltar está em
    /// [`EventCx::pointer`](crate::EventCx::pointer), como no [`Event::Drop`].
    ///
    /// Chega no Windows, no macOS e no X11. O `winit` 0.30 não entrega o soltar no Wayland, e
    /// nenhum sistema informa a posição do ponteiro durante o arrasto de fora: o ponto é o
    /// último que a janela viu.
    FilesDropped {
        /// Os caminhos, na ordem em que o sistema os entregou.
        paths: Vec<std::path::PathBuf>,
    },
}

/// Forma do ponteiro do mouse sobre um widget ([`Widget::cursor`](crate::Widget::cursor)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Cursor {
    /// A seta.
    #[default]
    Default,
    /// Setas para a esquerda e a direita: o divisor de um `Split::row`.
    EwResize,
    /// Setas para cima e para baixo: o divisor de um `Split::column`.
    NsResize,
    /// A mãozinha: um link.
    Hand,
}
