//! Árvore de widgets, foco e modelo de eventos.
//!
//! Cada widget (`Form`, `Column`, `Row`, `UniformGrid`, `Border`, `Card`, `SettingsCard`, `SettingsExpander`, `ScrollViewer`, `Text`, `Icon`, `Image`, `Button`, `ToggleButton`, `Edit`,
//! `CheckBox`, `ToggleSwitch`, `ComboBox`, `RadioButtons`, `Slider`, `ProgressBar`, `ProgressRing`,
//! `Expander`, `TabView`, `NavigationView`, `AutoSuggestBox`, `NumberBox`, `InfoBar`, `TitleBar`, `MenuBar` e `Split`) guarda só estado e lógica e tem um `id`
//! estável, usado em testes, no MCP e no lint. Os grupos (`RadioButtons`, `TabView`, `NavigationView`, `ComboBox`,
//! `AutoSuggestBox`, `NumberBox` e o `Edit` de senha) dão às partes que montam ids derivados do
//! deles: `tema-claro`, `config-geral`, `senha-revelar`. `Show` e `For` são fragmentos: trocam os filhos quando os
//! dados mudam, e a árvore cria e descarta só os widgets que entraram e saíram.
//!
//! Os popups (`ContentDialog`, `Flyout`, `MenuFlyout`, as listas do `ComboBox` e do
//! `AutoSuggestBox` e as dicas de `.tooltip()`) são camadas ([`Layer`]): ficam na árvore como filhos da origem, mas aparecem
//! acima de tudo, e a árvore cuida do foco, do Esc e do clique fora.
//!
//! Quem mede e pinta é o `Drawer` do crate de tema, a partir do [`Look`]; quem posiciona é o
//! crate de layout.

mod auto_suggest_box;
mod border;
mod button;
mod checkbox;
pub mod clipboard;
mod color;
mod combo_box;
mod containers;
mod cx;
mod data_grid;
mod density;
mod dialog;
pub mod dialogs;
mod dock;
mod drag;
mod edit;
pub mod event;
mod expander;
mod field_button;
mod flyout;
mod fragments;
mod icon;
mod image;
mod info_bar;
mod menu_bar;
pub mod native;
mod navigation_view;
mod number_box;
mod press;
pub mod process;
mod progress;
mod prop;
mod property_grid;
mod radio_buttons;
mod rich_text;
mod scroll_viewer;
mod settings_card;
mod settings_expander;
mod shortcut;
mod slider;
mod split;
mod strings;
mod symbol;
mod tab_view;
mod text;
mod text_editor;
mod title_bar;
mod toggle_button;
mod toggle_switch;
mod tooltip;
pub mod tray;
mod tree;
mod tree_view;
mod wake;
mod widget;
pub mod windows;

pub use auto_suggest_box::AutoSuggestBox;
pub use border::{
    Border, BorderVisual, CARD_BORDER, CARD_CORNER_RADIUS, CARD_PADDING, Card, CardVisual,
};
pub use button::{
    BUTTON_ICON_SIZE, BUTTON_ICON_SPACING, Button, ButtonState, ButtonVariant, ButtonVisual,
    ICON_BUTTON_SIZE, ICON_BUTTON_SIZE_COMPACT,
};
pub use checkbox::{CheckBox, CheckBoxVisual};
pub use color::{Brush, Color, GradientStop, LinearGradient, ThemeColor, Token};
pub use combo_box::{
    COMBO_BOX_ITEM_HEIGHT, COMBO_BOX_ITEM_HEIGHT_COMPACT, ComboBox, ComboBoxItemVisual,
    ComboBoxVisual,
};
pub use containers::{Column, Form, Row, UNIFORM_GRID_MIN_COLUMN_WIDTH, UniformGrid};
pub use cx::{
    Clipboard, ClipboardImage, EventCx, FixedMetrics, FocusMove, InputPurpose, MemoryClipboard,
    TextInput, TextMetrics,
};
pub use data_grid::{
    CellValue, ColumnWidth, DATA_GRID_CELL_PADDING, DATA_GRID_DIVIDER_HIT, DATA_GRID_HEADER_HEIGHT,
    DATA_GRID_HEADER_HEIGHT_COMPACT, DATA_GRID_ICON, DATA_GRID_ICON_GAP, DATA_GRID_MIN_COLUMN,
    DATA_GRID_ROW_HEIGHT, DATA_GRID_ROW_HEIGHT_COMPACT, DataGrid, DataGridColumn, DataGridDrop,
    DataGridFlat, DataGridHeaderVisual, DataGridRow, DataGridRowVisual, DataGridVisual, RowTone,
    SortDirection, data_grid_cells,
};
pub use density::Density;
pub use dialog::ContentDialog;
pub use dock::{
    DOCK_HEADER_HEIGHT, DOCK_HOVER_DELAY, DOCK_LEAVE_DELAY, DOCK_PANE_SIZE, DOCK_STRIP_HEIGHT,
    DOCK_STRIP_WIDTH, DockArea, DockController, DockHost, DockLayout, DockMode, DockPane,
    DockPaneState, DockSide,
};
pub use drag::{DRAG_THRESHOLD, DragData, InsertMark};
pub use edit::{Edit, EditState, EditVisual, PASSWORD_CHAR, PasswordReveal};
pub use event::{Cursor, Event, FocusSource, Ime, Key, Modifiers, PointerKind};
pub use expander::{
    EXPANDER_DESCRIBED_HEADER_HEIGHT, EXPANDER_HEADER_HEIGHT, Expander, ExpanderVisual,
};
pub use field_button::{FieldButtonVisual, FieldGlyph, SPIN_REPEAT};
pub use flyout::{ContextFlyout, Flyout, FlyoutContent, MenuFlyout, MenuItemKind, MenuItemVisual};
pub use fragments::{For, Show};
pub use icon::{ICON_SIZE, Icon, IconVisual};
pub use image::{Image, ImageSource, ImageVisual, Stretch};
pub use info_bar::{InfoBar, InfoBarSeverity, InfoBarVisual};
pub use menu_bar::{MENU_BAR_HEIGHT, MENU_BAR_HEIGHT_COMPACT, MenuBar, MenuBarItemVisual};
pub use native::{NativeFocus, NativePlace};
pub use navigation_view::{
    NAV_COMPACT_MIN_WIDTH, NAV_COMPACT_WIDTH, NAV_EXPANDED_MIN_WIDTH, NAV_INDENT, NAV_ITEM_HEIGHT,
    NAV_ITEM_HEIGHT_COMPACT, NAV_PANE_WIDTH, NavigationGroup, NavigationItemVisual, NavigationMode,
    NavigationView, NavigationViewVisual,
};
pub use number_box::{NumberBox, SpinButtons, format_number, parse_number};
pub use progress::{ProgressBar, ProgressRing, ProgressStatus, ProgressVisual};
pub use prop::{IntoProp, Prop, marker};
pub use property_grid::{
    CategoryInfo, PROPERTY_GRID_GRIP, PROPERTY_GRID_INDENT, PROPERTY_GRID_MIN_COLUMN,
    PROPERTY_GRID_NAME_X, PropertyGrid, PropertyGridCategoryVisual, PropertyGridRowVisual,
    PropertyGridSizerVisual, PropertyGridVisual, PropertyInfo, PropertyValue,
};
pub use radio_buttons::{
    RADIO_BUTTON_HEIGHT, RADIO_BUTTON_HEIGHT_COMPACT, RadioButtonVisual, RadioButtons,
};
pub use rich_text::{LinkRange, RichText, RichTextVisual, Span};
pub use scroll_viewer::{
    SCROLL_BAR_SIZE, SCROLL_LINE, SCROLL_THUMB_MIN, ScrollAxis, ScrollBarParts, ScrollBarState,
    ScrollLink, ScrollMode, ScrollViewer, ScrollViewerVisual, Scroller,
};
pub use settings_card::{
    SETTINGS_CARD_BORDER, SETTINGS_CARD_CORNER_RADIUS, SETTINGS_CARD_MIN_HEIGHT,
    SETTINGS_CARD_PADDING, SETTINGS_CARD_WRAP_NO_ICON_THRESHOLD, SETTINGS_CARD_WRAP_THRESHOLD,
    SETTINGS_EXPANDER_ITEM_MIN_HEIGHT, SettingsCard, SettingsCardPart, SettingsCardVisual,
};
pub use settings_expander::SettingsExpander;
pub use shortcut::{Accelerator, ParseShortcutError, Shortcut};
pub use slider::{SLIDER_THUMB_SIZE, Slider, SliderVisual};
pub use split::{SPLIT_KEY_STEP, SPLIT_SIZER, SizerVisual, Split};
pub use strings::{Strings, set_strings, strings};
pub use symbol::Symbol;
pub use tab_view::{
    TAB_ATTENTION_DOT, TAB_ATTENTION_GAP, TAB_BUTTON_SIZE, TAB_BUTTON_SIZE_COMPACT, TAB_CLOSE_GAP,
    TAB_HEIGHT, TAB_HEIGHT_COMPACT, TAB_ICON_GAP, TAB_ICON_SIZE, TAB_MAX_WIDTH, TAB_MIN_WIDTH,
    TAB_SCROLL_STEP, TAB_STRIP_HEIGHT, TAB_STRIP_HEIGHT_COMPACT, TabHeaderVisual, TabProgress,
    TabStripButtonKind, TabStripButtonVisual, TabView, TabViewVisual, TabWidth,
};
pub use text::{Text, TextRamp, TextVisual};
pub use text_editor::{
    EditorPoints, EditorText, KeyEvent, Mark, StyleRange, TextEditor, TextEditorController,
    TextEditorVisual, request_focus, request_focus_in, request_focus_inside, start_bring_into_view,
};
pub use title_bar::{
    CAPTION_BUTTON_WIDTH, CaptionButtonVisual, CaptionGlyph, DRAG_SLOP, PaneLink, RESIZE_BORDER,
    ResizeEdge, TITLE_BAR_BUTTON_WIDTH, TITLE_BAR_COMPACT_HEIGHT, TITLE_BAR_HEIGHT, TitleBadge,
    TitleBar, TitleBarButtonVisual, WindowChrome, WindowRequest, WindowState,
};
pub use toggle_button::{ToggleButton, ToggleButtonVisual};
pub use toggle_switch::{ToggleSwitch, ToggleSwitchVisual};
pub use tree::{ANIMATION_FRAME, Changes, HOLD_DELAY, NodeId, TOOLTIP_DELAY, Tree};
pub use tree_view::{
    DropMark, ItemStatus, TREE_VIEW_CHEVRON_COLUMN, TREE_VIEW_ICON, TREE_VIEW_ICON_GAP,
    TREE_VIEW_INDENT, TREE_VIEW_ITEM_HEIGHT, TREE_VIEW_ITEM_HEIGHT_COMPACT, TREE_VIEW_STATUS_BADGE,
    TreeFlat, TreeItem, TreeItems, TreeRow, TreeView, TreeViewItemVisual, TreeViewVisual,
};
pub use wake::{clear_waker, set_waker, wake};
pub use widget::{
    Align, Common, CustomLook, CustomVisual, Direction, Flex, Insets, Justify, Layer, Look,
    Placement, PopupKind, Slot, View, Widget, Within,
};
pub use windows::{
    NewWindow, WindowGeometry, WindowHandle, active_window, closing_all_windows,
    on_close_requested, open_window,
};

/// Geometria usada pela árvore (`Rect`, `Point`, `Size`, `Vec2`).
pub use kurbo;
