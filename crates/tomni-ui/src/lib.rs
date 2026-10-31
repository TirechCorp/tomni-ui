//! Fachada do Tomni UI: o único crate de que um app precisa depender.
//!
//! Além de reexportar as camadas, traz o [`Ui`], que liga árvore, layout, tema e render num
//! loop de frame, e o [`App`], que abre o `Ui` numa janela. O runner da plataforma e o
//! `TestApp` usam o mesmo `Ui`.

mod app;
mod inspect;
pub mod lint;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "tools")]
pub mod tools;
mod ui;

pub use app::App;
pub use inspect::{
    CategoryNodeInfo, GridRowInfo, MAX_TREE_ITEMS, Node, PaneNodeInfo, PropertyNodeInfo,
    TreeItemInfo,
};
pub use tomni_ui_platform::{Backdrop, LauncherEntry, LauncherState, WindowIcon, rgba};
pub use ui::Ui;

/// Fixa o tema de todas as janelas a partir de agora (as que abrirem depois nascem com ele);
/// `None` volta a seguir o claro ou escuro do sistema. Vale com o app aberto, sem remontar
/// nada, e sobrepõe o [`App::theme`].
///
/// ```
/// use tomni_ui::{set_theme, theme::Theme};
/// set_theme(Some(Theme::Dark));
/// set_theme(None);
/// ```
pub fn set_theme(theme: Option<theme::Theme>) {
    tomni_ui_core::windows::request_theme_override(theme.map(|t| t == theme::Theme::Dark));
}

// Widgets direto na raiz (`tomni_ui::Button`).
/// Modelos de linguagem (feature `ai`): o pedido em streaming nos dialetos compatível com OpenAI
/// e Messages da Anthropic, a lista de modelos e o servidor falso para os testes.
#[cfg(feature = "ai")]
pub use tomni_ui_ai as ai;
pub use tomni_ui_core::*;
/// Arquivos locais e remotos (feature `files`, e `sftp` para o SFTP): o `Fs`, a `Remote` e a
/// fila de transferências.
#[cfg(feature = "files")]
pub use tomni_ui_files as files;
pub use tomni_ui_layout as layout;
/// O Markdown (feature `markdown`): o `MarkdownView`, o realce e a edição para o `TextEditor`.
#[cfg(feature = "markdown")]
pub use tomni_ui_markdown as markdown;
/// O proxy e a conexão TCP que o SSH e o SFTP dividem (feature `files`).
#[cfg(feature = "files")]
pub use tomni_ui_net as net;
pub use tomni_ui_platform as platform;
pub use tomni_ui_reactive as reactive;
pub use tomni_ui_render as render;
/// O terminal (feature `terminal`): o widget, o emulador e os transportes.
#[cfg(feature = "terminal")]
pub use tomni_ui_terminal as terminal;
#[cfg(feature = "terminal")]
pub use tomni_ui_terminal::Terminal;
pub use tomni_ui_text as text;
pub use tomni_ui_theme as theme;
/// O WebView (feature `webview`, ou `webview-widget` só com o motor falso): o widget e o
/// controller.
#[cfg(feature = "webview-widget")]
pub use tomni_ui_webview as webview;
#[cfg(feature = "webview-widget")]
pub use tomni_ui_webview::{WebView, WebViewController};

/// Funções soltas sobre imagens, fora do widget [`Image`].
pub mod image {
    /// Largura e altura, em pixels, da imagem em `bytes` (PNG, JPEG, GIF, WebP ou SVG), ou a
    /// mensagem de erro, em português, se ela não abre.
    ///
    /// Decodifica pelo mesmo caminho do [`Image`](crate::Image), então uma imagem que passa aqui
    /// também é desenhada, e uma corrompida dá o erro antes de virar um quadro vazio na tela.
    /// Decodifica a imagem inteira: chame numa thread, não no frame, e passe o resultado ao
    /// [`Image::natural_size`](crate::Image::natural_size).
    ///
    /// ```
    /// let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"/>"#;
    /// assert_eq!(tomni_ui::image::probe(svg), Ok((40, 30)));
    /// assert!(tomni_ui::image::probe(b"nao e imagem").is_err());
    /// ```
    pub fn probe(bytes: &[u8]) -> Result<(u32, u32), String> {
        let size = tomni_ui_render::decode(bytes)
            .map_err(|e| e.message().to_owned())?
            .size();
        Ok((size.width.round() as u32, size.height.round() as u32))
    }
}

/// `use tomni_ui::prelude::*` traz signals, widgets e o necessário para montar uma view.
pub mod prelude {
    pub use tomni_ui_core::{
        Align, AreaChart, AutoSuggestBox, Border, Brush, Button, Card, CellValue, CheckBox, Color,
        Column, ColumnWidth, ComboBox, ContentDialog, DataGrid, DataGridColumn, DataGridDrop,
        DataGridRow, Density, DockController, DockHost, DockLayout, DockMode, DockPane, DockSide,
        Edit, Expander, Flyout, For, Form, HeatMap, Icon, Image, InfoBar, InfoBarSeverity, Insets,
        ItemStatus, Justify, Key, LevelMeter, MenuBar, MenuFlyout, NavigationMode, NavigationView,
        NumberBox, PasswordReveal, ProgressBar, ProgressRing, ProgressStatus, PropertyGrid,
        RadioButtons, RichText, Row, RowTone, ScrollViewer, SettingsCard, SettingsExpander,
        Shortcut, Show, Slider, SortDirection, Span, Sparkline, SpinButtons, Split, Stretch,
        Symbol, TabProgress, TabView, TabWidth, Text, TextEditor, TextEditorController, TextRamp,
        ThemeColor, TitleBadge, TitleBar, ToggleButton, ToggleSwitch, Token, TreeItem, TreeItems,
        TreeView, UniformGrid, View, WindowHandle, active_window, on_close_requested, open_window,
    };
    #[cfg(feature = "markdown")]
    pub use tomni_ui_markdown::MarkdownView;
    pub use tomni_ui_platform::{Backdrop, WindowIcon};
    pub use tomni_ui_reactive::prelude::*;
    #[cfg(feature = "terminal")]
    pub use tomni_ui_terminal::{Loopback, Scheme, Terminal};
    #[cfg(feature = "webview-widget")]
    pub use tomni_ui_webview::{LoadState, WebView, WebViewController};
}
