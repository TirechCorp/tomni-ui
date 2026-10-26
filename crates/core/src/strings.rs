//! Os textos próprios da lib (dicas dos botões, aviso de colar, painéis do terminal…), numa
//! tabela por thread que o app preenche com [`set_strings`](crate::set_strings).
//!
//! O padrão é o português do Brasil, os textos que a lib sempre teve: quem não chama
//! `set_strings` não vê diferença. A lib não conhece idiomas nem o Fluent; só guarda a tabela.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

/// Os textos que a lib escreve sozinha, um campo por texto.
///
/// É `#[non_exhaustive]`: monte a partir do [`Default`] (os textos em português) e troque os
/// campos que quiser. Um campo que a lib acrescentar depois fica em português até o app
/// preenchê-lo, e o app continua compilando.
///
/// ```
/// use tomni_ui_core::Strings;
/// let mut s = Strings::default();
/// s.tab_close = "Close tab".into();
/// assert_eq!(s.tab_add, "Nova aba");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Strings {
    /// Dica do ✕ da aba.
    pub tab_close: Cow<'static, str>,
    /// Dica do + da faixa de abas.
    pub tab_add: Cow<'static, str>,
    /// Seta esquerda da faixa de abas.
    pub tab_scroll_left: Cow<'static, str>,
    /// Seta direita da faixa de abas.
    pub tab_scroll_right: Cow<'static, str>,
    /// Botão voltar da barra de título.
    pub back: Cow<'static, str>,
    /// ☰ aberto (a ação é fechar), na barra de título e no painel de navegação.
    pub navigation_close: Cow<'static, str>,
    /// ☰ fechado (a ação é abrir).
    pub navigation_open: Cow<'static, str>,
    /// Lupa do painel de navegação.
    pub navigation_search: Cow<'static, str>,
    /// Item de configurações do rodapé do `NavigationView`.
    pub navigation_settings: Cow<'static, str>,
    /// Alfinete solto: fixa o painel ao lado do conteúdo.
    pub dock_pin: Cow<'static, str>,
    /// Alfinete fixado: o painel vira um botão na faixa da borda.
    pub dock_unpin: Cow<'static, str>,
    /// Passa o painel para uma janela própria.
    pub dock_float: Cow<'static, str>,
    /// Traz o painel flutuante de volta à janela.
    pub dock_dock: Cow<'static, str>,
    /// Fecha o painel do `DockHost`.
    pub dock_close: Cow<'static, str>,
    /// Maximiza o painel do `DockHost`.
    pub dock_maximize: Cow<'static, str>,
    /// Desfaz o maximizar do painel do `DockHost`.
    pub dock_restore: Cow<'static, str>,
    /// Menu do cabeçalho do painel.
    pub dock_options: Cow<'static, str>,
    /// Move o painel para a esquerda.
    pub dock_side_left: Cow<'static, str>,
    /// Move o painel para a direita.
    pub dock_side_right: Cow<'static, str>,
    /// Move o painel para baixo.
    pub dock_side_bottom: Cow<'static, str>,
    /// Botão minimizar da janela.
    pub minimize: Cow<'static, str>,
    /// Botão maximizar da janela.
    pub maximize: Cow<'static, str>,
    /// Botão restaurar da janela.
    pub restore: Cow<'static, str>,
    /// Botão fechar da janela.
    pub close: Cow<'static, str>,
    /// Estado de uma aba com andamento sem valor, no nome dela.
    pub tab_progress_working: Cow<'static, str>,
    /// Estado de uma aba cuja conexão ainda está abrindo, no nome dela.
    pub tab_connecting: Cow<'static, str>,
    /// Estado de uma aba com o andamento pausado, no nome dela.
    pub tab_progress_paused: Cow<'static, str>,
    /// Estado de uma aba com o andamento em erro, no nome dela.
    pub tab_progress_error: Cow<'static, str>,
    /// Estado de uma aba com o ponto de atenção aceso, no nome dela.
    pub tab_attention: Cow<'static, str>,
    /// Estado de um item da árvore com o selo de sucesso, no nome dele.
    pub tree_status_success: Cow<'static, str>,
    /// Estado de um item da árvore com o selo de cautela, no nome dele.
    pub tree_status_caution: Cow<'static, str>,
    /// Estado de um item da árvore com o selo de erro, no nome dele.
    pub tree_status_critical: Cow<'static, str>,
    /// Estado de um item da árvore com o selo neutro, no nome dele.
    pub tree_status_neutral: Cow<'static, str>,
    /// Botão de revelar a senha.
    pub show_password: Cow<'static, str>,
    /// Seta de aumentar do `NumberBox`.
    pub increase: Cow<'static, str>,
    /// Seta de diminuir do `NumberBox`.
    pub decrease: Cow<'static, str>,
    /// ✕ de limpar dos campos.
    pub clear: Cow<'static, str>,
    /// Botão de buscar do `AutoSuggestBox` (a lupa).
    pub query: Cow<'static, str>,
    /// Texto ao lado do `ToggleSwitch` ligado.
    pub toggle_on: Cow<'static, str>,
    /// Texto ao lado do `ToggleSwitch` desligado.
    pub toggle_off: Cow<'static, str>,
    /// Botão de ordem alfabética do `PropertyGrid`.
    pub sort_alphabetical: Cow<'static, str>,
    /// Título do aviso de colar várias linhas.
    pub paste_title: Cow<'static, str>,
    /// Texto do aviso de colar várias linhas.
    pub paste_warning: Cow<'static, str>,
    /// O mesmo aviso quando a colagem vai para vários terminais; `{count}` vira a quantidade.
    pub paste_warning_many: Cow<'static, str>,
    /// Botão que confirma a colagem.
    pub paste_anyway: Cow<'static, str>,
    /// Botão que desiste da colagem.
    pub cancel: Cow<'static, str>,
    /// Item do menu do terminal que copia a seleção, quando o menu é o do link.
    pub terminal_copy: Cow<'static, str>,
    /// Item do menu do terminal que cola, quando o menu é o do link.
    pub terminal_paste: Cow<'static, str>,
    /// Rótulo de abrir um link do terminal quando o app não dá outro (`link_action`).
    pub terminal_link_open: Cow<'static, str>,
    /// Item do menu do terminal que copia o destino do link.
    pub terminal_link_copy: Cow<'static, str>,
    /// Segunda linha da dica de um link; `{action}` vira o rótulo de abrir.
    pub terminal_link_hint: Cow<'static, str>,
    /// Segunda linha da dica de um link que não abre dali.
    pub terminal_link_no_open: Cow<'static, str>,
    /// Texto do painel sem abas.
    pub pane_empty: Cow<'static, str>,
    /// Botão e item de menu que fecham um painel vazio.
    pub pane_close: Cow<'static, str>,
    /// Nome de um painel nos menus; `{n}` vira o número, a partir de 1.
    pub pane_name: Cow<'static, str>,
    /// Submenu "mover a aba para o painel".
    pub pane_move_to: Cow<'static, str>,
    /// Item "novo painel à direita".
    pub pane_new_right: Cow<'static, str>,
    /// Item "novo painel abaixo".
    pub pane_new_below: Cow<'static, str>,
    /// Item "reiniciar sessão".
    pub pane_restart: Cow<'static, str>,
    /// Item "fechar aba" do menu da aba.
    pub pane_close_tab: Cow<'static, str>,
    /// O que se escreve no terminal quando a conexão cai sem motivo.
    pub connection_closed: Cow<'static, str>,
    /// O mesmo, com motivo; `{reason}` vira o motivo.
    pub connection_closed_reason: Cow<'static, str>,
    /// Título da falha do `WebView`.
    pub page_failed: Cow<'static, str>,
    /// Botão que tenta de novo no `WebView`.
    pub try_again: Cow<'static, str>,
}

impl Default for Strings {
    fn default() -> Self {
        Self {
            tab_close: "Fechar aba".into(),
            tab_add: "Nova aba".into(),
            tab_scroll_left: "Rolar para a esquerda".into(),
            tab_scroll_right: "Rolar para a direita".into(),
            back: "Voltar".into(),
            navigation_close: "Fechar navegação".into(),
            navigation_open: "Abrir navegação".into(),
            navigation_search: "Pesquisar".into(),
            navigation_settings: "Configurações".into(),
            dock_pin: "Fixar".into(),
            dock_unpin: "Ocultar automaticamente".into(),
            dock_float: "Flutuar".into(),
            dock_dock: "Encaixar".into(),
            dock_close: "Fechar o painel".into(),
            dock_maximize: "Maximizar".into(),
            dock_restore: "Restaurar".into(),
            dock_options: "Opções do painel".into(),
            dock_side_left: "Encaixar à esquerda".into(),
            dock_side_right: "Encaixar à direita".into(),
            dock_side_bottom: "Encaixar embaixo".into(),
            minimize: "Minimizar".into(),
            maximize: "Maximizar".into(),
            restore: "Restaurar".into(),
            close: "Fechar".into(),
            tab_progress_working: "trabalhando".into(),
            tab_connecting: "conectando".into(),
            tab_progress_paused: "pausado".into(),
            tab_progress_error: "erro".into(),
            tab_attention: "pede atenção".into(),
            tree_status_success: "conectado".into(),
            tree_status_caution: "conectando".into(),
            tree_status_critical: "com erro".into(),
            tree_status_neutral: "encerrado".into(),
            show_password: "Mostrar senha".into(),
            increase: "Aumentar".into(),
            decrease: "Diminuir".into(),
            clear: "Limpar".into(),
            query: "Buscar".into(),
            toggle_on: "Ligado".into(),
            toggle_off: "Desligado".into(),
            sort_alphabetical: "Ordem alfabética".into(),
            paste_title: "Aviso".into(),
            paste_warning: "Você está prestes a colar um texto com várias linhas. Colado num \
                            shell, ele pode executar comandos sem querer. Deseja continuar?"
                .into(),
            paste_warning_many: "Colar em {count} terminais? O texto vai para todos os que \
                                 recebem o que você digita, e colado num shell ele pode \
                                 executar comandos sem querer."
                .into(),
            paste_anyway: "Colar mesmo assim".into(),
            cancel: "Cancelar".into(),
            terminal_copy: "Copiar".into(),
            terminal_paste: "Colar".into(),
            terminal_link_open: "Abrir link".into(),
            terminal_link_copy: "Copiar link".into(),
            terminal_link_hint: "Ctrl+clique: {action}".into(),
            terminal_link_no_open: "Este link não abre daqui; use Copiar link no menu".into(),
            pane_empty: "Arraste uma aba para cá".into(),
            pane_close: "Fechar o painel".into(),
            pane_name: "Painel {n}".into(),
            pane_move_to: "Mover para o painel".into(),
            pane_new_right: "Novo painel à direita".into(),
            pane_new_below: "Novo painel abaixo".into(),
            pane_restart: "Reiniciar sessão".into(),
            pane_close_tab: "Fechar aba".into(),
            connection_closed: "[conexão encerrada]".into(),
            connection_closed_reason: "[conexão encerrada: {reason}]".into(),
            page_failed: "Não foi possível abrir a página".into(),
            try_again: "Tentar de novo".into(),
        }
    }
}

thread_local! {
    static STRINGS: RefCell<Rc<Strings>> = RefCell::new(Rc::new(Strings::default()));
}

/// Troca a tabela de textos desta thread (a da interface). Vale para os widgets montados
/// **depois** da chamada: chame antes de montar a tela, no início do `main`. Os já montados podem
/// ficar com o texto velho.
pub fn set_strings(strings: Strings) {
    STRINGS.with(|s| *s.borrow_mut() = Rc::new(strings));
}

/// A tabela de textos desta thread, para os crates da lib lerem.
#[doc(hidden)]
pub fn strings() -> Rc<Strings> {
    STRINGS.with(|s| Rc::clone(&s.borrow()))
}
