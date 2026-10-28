//! Os ícones do `Icon` e dos botões, dos Fluent System Icons (MIT, Microsoft).
//!
//! Gerado por `crates/theme/icons/gerar.py` a partir de `crates/theme/icons/lista.txt`;
//! não edite à mão.

/// Um ícone dos Fluent System Icons.
///
/// Os nomes seguem os do Fluent (`ic_fluent_search_20_regular` vira `Symbol::Search`), não
/// os do enum `Symbol` do WinUI, que vêm da fonte Segoe Fluent Icons. As variantes
/// `*Filled` são os desenhos cheios. A lista com as categorias está no guia dos apps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Symbol {
    // Navegação
    /// `home_regular`.
    Home,
    /// `arrow_left_regular`.
    ArrowLeft,
    /// `arrow_right_regular`.
    ArrowRight,
    /// `arrow_up_regular`.
    ArrowUp,
    /// `arrow_down_regular`.
    ArrowDown,
    /// `chevron_left_regular`.
    ChevronLeft,
    /// `chevron_right_regular`.
    ChevronRight,
    /// `chevron_up_regular`.
    ChevronUp,
    /// `chevron_down_regular`.
    ChevronDown,
    /// `navigation_regular`.
    Navigation,
    /// `more_horizontal_regular`.
    MoreHorizontal,
    /// `more_vertical_regular`.
    MoreVertical,
    /// `open_regular`.
    Open,
    /// `arrow_clockwise_regular`.
    ArrowClockwise,
    /// `arrow_counterclockwise_regular`.
    ArrowCounterclockwise,
    /// `arrow_sync_regular`.
    ArrowSync,
    /// `arrow_undo_regular`.
    ArrowUndo,
    /// `arrow_redo_regular`.
    ArrowRedo,
    /// `arrow_exit_regular`.
    ArrowExit,
    /// `sign_out_regular`.
    SignOut,
    /// `arrow_enter_regular`.
    ArrowEnter,
    /// `grid_regular`.
    Grid,
    /// `apps_regular`.
    Apps,
    /// `apps_list_regular`.
    AppsList,
    /// `panel_left_regular`.
    PanelLeft,
    /// `panel_right_regular`.
    PanelRight,
    /// `panel_bottom_regular`.
    PanelBottom,
    /// `split_vertical_regular`.
    SplitVertical,
    /// `split_horizontal_regular`.
    SplitHorizontal,
    /// `globe_regular`.
    Globe,
    /// `map_regular`.
    Map,
    /// `location_regular`.
    Location,
    /// `compass_northwest_regular`.
    Compass,
    /// `search_regular`.
    Search,
    /// `filter_regular`.
    Filter,
    /// `arrow_sort_regular`.
    ArrowSort,
    /// `zoom_in_regular`.
    ZoomIn,
    /// `zoom_out_regular`.
    ZoomOut,
    /// `full_screen_maximize_regular`.
    FullScreenMaximize,
    /// `full_screen_minimize_regular`.
    FullScreenMinimize,
    /// `maximize_regular`.
    Maximize,
    /// `dismiss_regular`.
    Dismiss,
    // Ações
    /// `add_regular`.
    Add,
    /// `subtract_regular`.
    Subtract,
    /// `edit_regular`.
    Edit,
    /// `delete_regular`.
    Delete,
    /// `save_regular`.
    Save,
    /// `copy_regular`.
    Copy,
    /// `cut_regular`.
    Cut,
    /// `clipboard_paste_regular`.
    ClipboardPaste,
    /// `share_regular`.
    Share,
    /// `send_regular`.
    Send,
    /// `print_regular`.
    Print,
    /// `checkmark_regular`.
    Checkmark,
    /// `pin_regular`.
    Pin,
    /// `pin_off_regular`.
    PinOff,
    /// `star_regular`.
    Star,
    /// `star_filled`.
    StarFilled,
    /// `star_off_regular`.
    StarOff,
    /// `heart_regular`.
    Heart,
    /// `heart_filled`.
    HeartFilled,
    /// `bookmark_regular`.
    Bookmark,
    /// `bookmark_filled`.
    BookmarkFilled,
    /// `flag_regular`.
    Flag,
    /// `tag_regular`.
    Tag,
    /// `link_regular`.
    Link,
    /// `attach_regular`.
    Attach,
    /// `archive_regular`.
    Archive,
    /// `rename_regular`.
    Rename,
    /// `lock_closed_regular`.
    LockClosed,
    /// `lock_open_regular`.
    LockOpen,
    /// `key_regular`.
    Key,
    /// `eye_regular`.
    Eye,
    /// `eye_off_regular`.
    EyeOff,
    /// `options_regular`.
    Options,
    /// `arrow_reply_regular`.
    ArrowReply,
    /// `arrow_forward_regular`.
    ArrowForward,
    /// `arrow_download_regular`.
    ArrowDownload,
    /// `arrow_upload_regular`.
    ArrowUpload,
    /// `cloud_regular`.
    Cloud,
    /// `cloud_arrow_up_regular`.
    CloudArrowUp,
    /// `cloud_arrow_down_regular`.
    CloudArrowDown,
    /// `select_all_on_regular`.
    SelectAll,
    // Arquivos e conteúdo
    /// `document_regular`.
    Document,
    /// `document_add_regular`.
    DocumentAdd,
    /// `document_text_regular`.
    DocumentText,
    /// `folder_regular`.
    Folder,
    /// `folder_open_regular`.
    FolderOpen,
    /// `folder_add_regular`.
    FolderAdd,
    /// `image_regular`.
    Image,
    /// `image_multiple_regular`.
    ImageMultiple,
    /// `video_regular`.
    Video,
    /// `music_note_2_regular`.
    MusicNote,
    /// `code_regular`.
    Code,
    /// `database_regular`.
    Database,
    /// `server_regular`.
    Server,
    /// `clipboard_regular`.
    Clipboard,
    /// `notepad_regular`.
    Notepad,
    /// `book_regular`.
    Book,
    /// `library_regular`.
    Library,
    /// `briefcase_regular`.
    Briefcase,
    /// `box_regular`.
    Box,
    // Status
    /// `info_regular`.
    Info,
    /// `warning_regular`.
    Warning,
    /// `error_circle_regular`.
    ErrorCircle,
    /// `checkmark_circle_regular`.
    CheckmarkCircle,
    /// `dismiss_circle_regular`.
    DismissCircle,
    /// `question_circle_regular`.
    QuestionCircle,
    /// `alert_regular`.
    Alert,
    /// `alert_off_regular`.
    AlertOff,
    /// `shield_regular`.
    Shield,
    /// `shield_checkmark_regular`.
    ShieldCheckmark,
    /// `clock_regular`.
    Clock,
    /// `history_regular`.
    History,
    /// `calendar_regular`.
    Calendar,
    /// `hourglass_regular`.
    Hourglass,
    /// `circle_regular`.
    Circle,
    /// `circle_filled`.
    CircleFilled,
    /// `sparkle_regular`.
    Sparkle,
    /// `lightbulb_regular`.
    Lightbulb,
    /// `bug_regular`.
    Bug,
    /// `wifi_1_regular`.
    Wifi,
    /// `battery_10_regular`.
    Battery,
    /// `flash_regular`.
    Flash,
    /// `data_area_regular`.
    DataArea,
    // Mídia
    /// `play_regular`.
    Play,
    /// `pause_regular`.
    Pause,
    /// `stop_regular`.
    Stop,
    /// `previous_regular`.
    Previous,
    /// `next_regular`.
    Next,
    /// `record_regular`.
    Record,
    /// `speaker_2_regular`.
    Speaker,
    /// `speaker_mute_regular`.
    SpeakerMute,
    /// `mic_regular`.
    Mic,
    /// `mic_off_regular`.
    MicOff,
    /// `camera_regular`.
    Camera,
    /// `headphones_regular`.
    Headphones,
    /// `fast_forward_regular`.
    FastForward,
    /// `rewind_regular`.
    Rewind,
    /// `live_regular`.
    Broadcast,
    // Sistema e pessoas
    /// `settings_regular`.
    Settings,
    /// `person_regular`.
    Person,
    /// `people_regular`.
    People,
    /// `person_add_regular`.
    PersonAdd,
    /// `person_circle_regular`.
    PersonCircle,
    /// `color_regular`.
    Color,
    /// `paint_brush_regular`.
    PaintBrush,
    /// `dark_theme_regular`.
    DarkTheme,
    /// `weather_moon_regular`.
    WeatherMoon,
    /// `weather_sunny_regular`.
    WeatherSunny,
    /// `desktop_regular`.
    Desktop,
    /// `laptop_regular`.
    Laptop,
    /// `phone_regular`.
    Phone,
    /// `tablet_regular`.
    Tablet,
    /// `keyboard_regular`.
    Keyboard,
    /// `window_regular`.
    Window,
    /// `window_new_regular`.
    WindowNew,
    /// `plug_connected_regular`.
    PlugConnected,
    /// `power_regular`.
    Power,
    /// `accessibility_regular`.
    Accessibility,
    /// `local_language_regular`.
    LocalLanguage,
    /// `text_font_regular`.
    TextFont,
    /// `wrench_regular`.
    Wrench,
    /// `toolbox_regular`.
    Toolbox,
    // Comunicação
    /// `mail_regular`.
    Mail,
    /// `chat_regular`.
    Chat,
    /// `chat_multiple_regular`.
    ChatMultiple,
    /// `comment_regular`.
    Comment,
    /// `call_regular`.
    Call,
    /// `megaphone_regular`.
    Megaphone,
    // Texto
    /// `text_bold_regular`.
    TextBold,
    /// `text_italic_regular`.
    TextItalic,
    /// `text_underline_regular`.
    TextUnderline,
    /// `text_strikethrough_regular`.
    TextStrikethrough,
    /// `text_align_left_regular`.
    TextAlignLeft,
    /// `text_align_center_regular`.
    TextAlignCenter,
    /// `text_align_right_regular`.
    TextAlignRight,
    /// `text_bullet_list_ltr_regular`.
    TextBulletList,
    /// `text_number_list_ltr_regular`.
    TextNumberList,
    /// `text_header_1_regular`.
    TextHeader1,
    /// `table_regular`.
    Table,
    /// `task_list_ltr_regular`.
    TaskListLtr,
    /// `text_quote_regular`.
    TextQuote,
    /// `line_horizontal_1_regular`.
    LineHorizontal1,
    /// `code_block_regular`.
    CodeBlock,
    // Desenvolvimento
    /// `branch_regular`.
    Branch,
    /// `window_console_regular`.
    WindowConsole,
    /// `puzzle_piece_regular`.
    PuzzlePiece,
    /// `beaker_regular`.
    Beaker,
    // Diversos
    /// `cart_regular`.
    Cart,
    /// `money_regular`.
    Money,
    /// `gift_regular`.
    Gift,
    /// `trophy_regular`.
    Trophy,
    /// `rocket_regular`.
    Rocket,
    /// `emoji_regular`.
    Emoji,
    /// `thumb_like_regular`.
    ThumbLike,
    /// `thumb_like_filled`.
    ThumbLikeFilled,
    /// `thumb_dislike_regular`.
    ThumbDislike,
}

impl Symbol {
    /// Todos os ícones, na ordem do guia.
    pub const ALL: [Symbol; 198] = [
        Symbol::Home,
        Symbol::ArrowLeft,
        Symbol::ArrowRight,
        Symbol::ArrowUp,
        Symbol::ArrowDown,
        Symbol::ChevronLeft,
        Symbol::ChevronRight,
        Symbol::ChevronUp,
        Symbol::ChevronDown,
        Symbol::Navigation,
        Symbol::MoreHorizontal,
        Symbol::MoreVertical,
        Symbol::Open,
        Symbol::ArrowClockwise,
        Symbol::ArrowCounterclockwise,
        Symbol::ArrowSync,
        Symbol::ArrowUndo,
        Symbol::ArrowRedo,
        Symbol::ArrowExit,
        Symbol::SignOut,
        Symbol::ArrowEnter,
        Symbol::Grid,
        Symbol::Apps,
        Symbol::AppsList,
        Symbol::PanelLeft,
        Symbol::PanelRight,
        Symbol::PanelBottom,
        Symbol::SplitVertical,
        Symbol::SplitHorizontal,
        Symbol::Globe,
        Symbol::Map,
        Symbol::Location,
        Symbol::Compass,
        Symbol::Search,
        Symbol::Filter,
        Symbol::ArrowSort,
        Symbol::ZoomIn,
        Symbol::ZoomOut,
        Symbol::FullScreenMaximize,
        Symbol::FullScreenMinimize,
        Symbol::Maximize,
        Symbol::Dismiss,
        Symbol::Add,
        Symbol::Subtract,
        Symbol::Edit,
        Symbol::Delete,
        Symbol::Save,
        Symbol::Copy,
        Symbol::Cut,
        Symbol::ClipboardPaste,
        Symbol::Share,
        Symbol::Send,
        Symbol::Print,
        Symbol::Checkmark,
        Symbol::Pin,
        Symbol::PinOff,
        Symbol::Star,
        Symbol::StarFilled,
        Symbol::StarOff,
        Symbol::Heart,
        Symbol::HeartFilled,
        Symbol::Bookmark,
        Symbol::BookmarkFilled,
        Symbol::Flag,
        Symbol::Tag,
        Symbol::Link,
        Symbol::Attach,
        Symbol::Archive,
        Symbol::Rename,
        Symbol::LockClosed,
        Symbol::LockOpen,
        Symbol::Key,
        Symbol::Eye,
        Symbol::EyeOff,
        Symbol::Options,
        Symbol::ArrowReply,
        Symbol::ArrowForward,
        Symbol::ArrowDownload,
        Symbol::ArrowUpload,
        Symbol::Cloud,
        Symbol::CloudArrowUp,
        Symbol::CloudArrowDown,
        Symbol::SelectAll,
        Symbol::Document,
        Symbol::DocumentAdd,
        Symbol::DocumentText,
        Symbol::Folder,
        Symbol::FolderOpen,
        Symbol::FolderAdd,
        Symbol::Image,
        Symbol::ImageMultiple,
        Symbol::Video,
        Symbol::MusicNote,
        Symbol::Code,
        Symbol::Database,
        Symbol::Server,
        Symbol::Clipboard,
        Symbol::Notepad,
        Symbol::Book,
        Symbol::Library,
        Symbol::Briefcase,
        Symbol::Box,
        Symbol::Info,
        Symbol::Warning,
        Symbol::ErrorCircle,
        Symbol::CheckmarkCircle,
        Symbol::DismissCircle,
        Symbol::QuestionCircle,
        Symbol::Alert,
        Symbol::AlertOff,
        Symbol::Shield,
        Symbol::ShieldCheckmark,
        Symbol::Clock,
        Symbol::History,
        Symbol::Calendar,
        Symbol::Hourglass,
        Symbol::Circle,
        Symbol::CircleFilled,
        Symbol::Sparkle,
        Symbol::Lightbulb,
        Symbol::Bug,
        Symbol::Wifi,
        Symbol::Battery,
        Symbol::Flash,
        Symbol::DataArea,
        Symbol::Play,
        Symbol::Pause,
        Symbol::Stop,
        Symbol::Previous,
        Symbol::Next,
        Symbol::Record,
        Symbol::Speaker,
        Symbol::SpeakerMute,
        Symbol::Mic,
        Symbol::MicOff,
        Symbol::Camera,
        Symbol::Headphones,
        Symbol::FastForward,
        Symbol::Rewind,
        Symbol::Broadcast,
        Symbol::Settings,
        Symbol::Person,
        Symbol::People,
        Symbol::PersonAdd,
        Symbol::PersonCircle,
        Symbol::Color,
        Symbol::PaintBrush,
        Symbol::DarkTheme,
        Symbol::WeatherMoon,
        Symbol::WeatherSunny,
        Symbol::Desktop,
        Symbol::Laptop,
        Symbol::Phone,
        Symbol::Tablet,
        Symbol::Keyboard,
        Symbol::Window,
        Symbol::WindowNew,
        Symbol::PlugConnected,
        Symbol::Power,
        Symbol::Accessibility,
        Symbol::LocalLanguage,
        Symbol::TextFont,
        Symbol::Wrench,
        Symbol::Toolbox,
        Symbol::Mail,
        Symbol::Chat,
        Symbol::ChatMultiple,
        Symbol::Comment,
        Symbol::Call,
        Symbol::Megaphone,
        Symbol::TextBold,
        Symbol::TextItalic,
        Symbol::TextUnderline,
        Symbol::TextStrikethrough,
        Symbol::TextAlignLeft,
        Symbol::TextAlignCenter,
        Symbol::TextAlignRight,
        Symbol::TextBulletList,
        Symbol::TextNumberList,
        Symbol::TextHeader1,
        Symbol::Table,
        Symbol::TaskListLtr,
        Symbol::TextQuote,
        Symbol::LineHorizontal1,
        Symbol::CodeBlock,
        Symbol::Branch,
        Symbol::WindowConsole,
        Symbol::PuzzlePiece,
        Symbol::Beaker,
        Symbol::Cart,
        Symbol::Money,
        Symbol::Gift,
        Symbol::Trophy,
        Symbol::Rocket,
        Symbol::Emoji,
        Symbol::ThumbLike,
        Symbol::ThumbLikeFilled,
        Symbol::ThumbDislike,
    ];

    /// O nome do ícone como se escreve no código (`"Search"`), o que a árvore mostra.
    pub fn name(self) -> &'static str {
        match self {
            Symbol::Home => "Home",
            Symbol::ArrowLeft => "ArrowLeft",
            Symbol::ArrowRight => "ArrowRight",
            Symbol::ArrowUp => "ArrowUp",
            Symbol::ArrowDown => "ArrowDown",
            Symbol::ChevronLeft => "ChevronLeft",
            Symbol::ChevronRight => "ChevronRight",
            Symbol::ChevronUp => "ChevronUp",
            Symbol::ChevronDown => "ChevronDown",
            Symbol::Navigation => "Navigation",
            Symbol::MoreHorizontal => "MoreHorizontal",
            Symbol::MoreVertical => "MoreVertical",
            Symbol::Open => "Open",
            Symbol::ArrowClockwise => "ArrowClockwise",
            Symbol::ArrowCounterclockwise => "ArrowCounterclockwise",
            Symbol::ArrowSync => "ArrowSync",
            Symbol::ArrowUndo => "ArrowUndo",
            Symbol::ArrowRedo => "ArrowRedo",
            Symbol::ArrowExit => "ArrowExit",
            Symbol::SignOut => "SignOut",
            Symbol::ArrowEnter => "ArrowEnter",
            Symbol::Grid => "Grid",
            Symbol::Apps => "Apps",
            Symbol::AppsList => "AppsList",
            Symbol::PanelLeft => "PanelLeft",
            Symbol::PanelRight => "PanelRight",
            Symbol::PanelBottom => "PanelBottom",
            Symbol::SplitVertical => "SplitVertical",
            Symbol::SplitHorizontal => "SplitHorizontal",
            Symbol::Globe => "Globe",
            Symbol::Map => "Map",
            Symbol::Location => "Location",
            Symbol::Compass => "Compass",
            Symbol::Search => "Search",
            Symbol::Filter => "Filter",
            Symbol::ArrowSort => "ArrowSort",
            Symbol::ZoomIn => "ZoomIn",
            Symbol::ZoomOut => "ZoomOut",
            Symbol::FullScreenMaximize => "FullScreenMaximize",
            Symbol::FullScreenMinimize => "FullScreenMinimize",
            Symbol::Maximize => "Maximize",
            Symbol::Dismiss => "Dismiss",
            Symbol::Add => "Add",
            Symbol::Subtract => "Subtract",
            Symbol::Edit => "Edit",
            Symbol::Delete => "Delete",
            Symbol::Save => "Save",
            Symbol::Copy => "Copy",
            Symbol::Cut => "Cut",
            Symbol::ClipboardPaste => "ClipboardPaste",
            Symbol::Share => "Share",
            Symbol::Send => "Send",
            Symbol::Print => "Print",
            Symbol::Checkmark => "Checkmark",
            Symbol::Pin => "Pin",
            Symbol::PinOff => "PinOff",
            Symbol::Star => "Star",
            Symbol::StarFilled => "StarFilled",
            Symbol::StarOff => "StarOff",
            Symbol::Heart => "Heart",
            Symbol::HeartFilled => "HeartFilled",
            Symbol::Bookmark => "Bookmark",
            Symbol::BookmarkFilled => "BookmarkFilled",
            Symbol::Flag => "Flag",
            Symbol::Tag => "Tag",
            Symbol::Link => "Link",
            Symbol::Attach => "Attach",
            Symbol::Archive => "Archive",
            Symbol::Rename => "Rename",
            Symbol::LockClosed => "LockClosed",
            Symbol::LockOpen => "LockOpen",
            Symbol::Key => "Key",
            Symbol::Eye => "Eye",
            Symbol::EyeOff => "EyeOff",
            Symbol::Options => "Options",
            Symbol::ArrowReply => "ArrowReply",
            Symbol::ArrowForward => "ArrowForward",
            Symbol::ArrowDownload => "ArrowDownload",
            Symbol::ArrowUpload => "ArrowUpload",
            Symbol::Cloud => "Cloud",
            Symbol::CloudArrowUp => "CloudArrowUp",
            Symbol::CloudArrowDown => "CloudArrowDown",
            Symbol::SelectAll => "SelectAll",
            Symbol::Document => "Document",
            Symbol::DocumentAdd => "DocumentAdd",
            Symbol::DocumentText => "DocumentText",
            Symbol::Folder => "Folder",
            Symbol::FolderOpen => "FolderOpen",
            Symbol::FolderAdd => "FolderAdd",
            Symbol::Image => "Image",
            Symbol::ImageMultiple => "ImageMultiple",
            Symbol::Video => "Video",
            Symbol::MusicNote => "MusicNote",
            Symbol::Code => "Code",
            Symbol::Database => "Database",
            Symbol::Server => "Server",
            Symbol::Clipboard => "Clipboard",
            Symbol::Notepad => "Notepad",
            Symbol::Book => "Book",
            Symbol::Library => "Library",
            Symbol::Briefcase => "Briefcase",
            Symbol::Box => "Box",
            Symbol::Info => "Info",
            Symbol::Warning => "Warning",
            Symbol::ErrorCircle => "ErrorCircle",
            Symbol::CheckmarkCircle => "CheckmarkCircle",
            Symbol::DismissCircle => "DismissCircle",
            Symbol::QuestionCircle => "QuestionCircle",
            Symbol::Alert => "Alert",
            Symbol::AlertOff => "AlertOff",
            Symbol::Shield => "Shield",
            Symbol::ShieldCheckmark => "ShieldCheckmark",
            Symbol::Clock => "Clock",
            Symbol::History => "History",
            Symbol::Calendar => "Calendar",
            Symbol::Hourglass => "Hourglass",
            Symbol::Circle => "Circle",
            Symbol::CircleFilled => "CircleFilled",
            Symbol::Sparkle => "Sparkle",
            Symbol::Lightbulb => "Lightbulb",
            Symbol::Bug => "Bug",
            Symbol::Wifi => "Wifi",
            Symbol::Battery => "Battery",
            Symbol::Flash => "Flash",
            Symbol::DataArea => "DataArea",
            Symbol::Play => "Play",
            Symbol::Pause => "Pause",
            Symbol::Stop => "Stop",
            Symbol::Previous => "Previous",
            Symbol::Next => "Next",
            Symbol::Record => "Record",
            Symbol::Speaker => "Speaker",
            Symbol::SpeakerMute => "SpeakerMute",
            Symbol::Mic => "Mic",
            Symbol::MicOff => "MicOff",
            Symbol::Camera => "Camera",
            Symbol::Headphones => "Headphones",
            Symbol::FastForward => "FastForward",
            Symbol::Rewind => "Rewind",
            Symbol::Broadcast => "Broadcast",
            Symbol::Settings => "Settings",
            Symbol::Person => "Person",
            Symbol::People => "People",
            Symbol::PersonAdd => "PersonAdd",
            Symbol::PersonCircle => "PersonCircle",
            Symbol::Color => "Color",
            Symbol::PaintBrush => "PaintBrush",
            Symbol::DarkTheme => "DarkTheme",
            Symbol::WeatherMoon => "WeatherMoon",
            Symbol::WeatherSunny => "WeatherSunny",
            Symbol::Desktop => "Desktop",
            Symbol::Laptop => "Laptop",
            Symbol::Phone => "Phone",
            Symbol::Tablet => "Tablet",
            Symbol::Keyboard => "Keyboard",
            Symbol::Window => "Window",
            Symbol::WindowNew => "WindowNew",
            Symbol::PlugConnected => "PlugConnected",
            Symbol::Power => "Power",
            Symbol::Accessibility => "Accessibility",
            Symbol::LocalLanguage => "LocalLanguage",
            Symbol::TextFont => "TextFont",
            Symbol::Wrench => "Wrench",
            Symbol::Toolbox => "Toolbox",
            Symbol::Mail => "Mail",
            Symbol::Chat => "Chat",
            Symbol::ChatMultiple => "ChatMultiple",
            Symbol::Comment => "Comment",
            Symbol::Call => "Call",
            Symbol::Megaphone => "Megaphone",
            Symbol::TextBold => "TextBold",
            Symbol::TextItalic => "TextItalic",
            Symbol::TextUnderline => "TextUnderline",
            Symbol::TextStrikethrough => "TextStrikethrough",
            Symbol::TextAlignLeft => "TextAlignLeft",
            Symbol::TextAlignCenter => "TextAlignCenter",
            Symbol::TextAlignRight => "TextAlignRight",
            Symbol::TextBulletList => "TextBulletList",
            Symbol::TextNumberList => "TextNumberList",
            Symbol::TextHeader1 => "TextHeader1",
            Symbol::Table => "Table",
            Symbol::TaskListLtr => "TaskListLtr",
            Symbol::TextQuote => "TextQuote",
            Symbol::LineHorizontal1 => "LineHorizontal1",
            Symbol::CodeBlock => "CodeBlock",
            Symbol::Branch => "Branch",
            Symbol::WindowConsole => "WindowConsole",
            Symbol::PuzzlePiece => "PuzzlePiece",
            Symbol::Beaker => "Beaker",
            Symbol::Cart => "Cart",
            Symbol::Money => "Money",
            Symbol::Gift => "Gift",
            Symbol::Trophy => "Trophy",
            Symbol::Rocket => "Rocket",
            Symbol::Emoji => "Emoji",
            Symbol::ThumbLike => "ThumbLike",
            Symbol::ThumbLikeFilled => "ThumbLikeFilled",
            Symbol::ThumbDislike => "ThumbDislike",
        }
    }

    /// O nome no Fluent, sem o tamanho (`"search_regular"`).
    pub fn fluent_name(self) -> &'static str {
        match self {
            Symbol::Home => "home_regular",
            Symbol::ArrowLeft => "arrow_left_regular",
            Symbol::ArrowRight => "arrow_right_regular",
            Symbol::ArrowUp => "arrow_up_regular",
            Symbol::ArrowDown => "arrow_down_regular",
            Symbol::ChevronLeft => "chevron_left_regular",
            Symbol::ChevronRight => "chevron_right_regular",
            Symbol::ChevronUp => "chevron_up_regular",
            Symbol::ChevronDown => "chevron_down_regular",
            Symbol::Navigation => "navigation_regular",
            Symbol::MoreHorizontal => "more_horizontal_regular",
            Symbol::MoreVertical => "more_vertical_regular",
            Symbol::Open => "open_regular",
            Symbol::ArrowClockwise => "arrow_clockwise_regular",
            Symbol::ArrowCounterclockwise => "arrow_counterclockwise_regular",
            Symbol::ArrowSync => "arrow_sync_regular",
            Symbol::ArrowUndo => "arrow_undo_regular",
            Symbol::ArrowRedo => "arrow_redo_regular",
            Symbol::ArrowExit => "arrow_exit_regular",
            Symbol::SignOut => "sign_out_regular",
            Symbol::ArrowEnter => "arrow_enter_regular",
            Symbol::Grid => "grid_regular",
            Symbol::Apps => "apps_regular",
            Symbol::AppsList => "apps_list_regular",
            Symbol::PanelLeft => "panel_left_regular",
            Symbol::PanelRight => "panel_right_regular",
            Symbol::PanelBottom => "panel_bottom_regular",
            Symbol::SplitVertical => "split_vertical_regular",
            Symbol::SplitHorizontal => "split_horizontal_regular",
            Symbol::Globe => "globe_regular",
            Symbol::Map => "map_regular",
            Symbol::Location => "location_regular",
            Symbol::Compass => "compass_northwest_regular",
            Symbol::Search => "search_regular",
            Symbol::Filter => "filter_regular",
            Symbol::ArrowSort => "arrow_sort_regular",
            Symbol::ZoomIn => "zoom_in_regular",
            Symbol::ZoomOut => "zoom_out_regular",
            Symbol::FullScreenMaximize => "full_screen_maximize_regular",
            Symbol::FullScreenMinimize => "full_screen_minimize_regular",
            Symbol::Maximize => "maximize_regular",
            Symbol::Dismiss => "dismiss_regular",
            Symbol::Add => "add_regular",
            Symbol::Subtract => "subtract_regular",
            Symbol::Edit => "edit_regular",
            Symbol::Delete => "delete_regular",
            Symbol::Save => "save_regular",
            Symbol::Copy => "copy_regular",
            Symbol::Cut => "cut_regular",
            Symbol::ClipboardPaste => "clipboard_paste_regular",
            Symbol::Share => "share_regular",
            Symbol::Send => "send_regular",
            Symbol::Print => "print_regular",
            Symbol::Checkmark => "checkmark_regular",
            Symbol::Pin => "pin_regular",
            Symbol::PinOff => "pin_off_regular",
            Symbol::Star => "star_regular",
            Symbol::StarFilled => "star_filled",
            Symbol::StarOff => "star_off_regular",
            Symbol::Heart => "heart_regular",
            Symbol::HeartFilled => "heart_filled",
            Symbol::Bookmark => "bookmark_regular",
            Symbol::BookmarkFilled => "bookmark_filled",
            Symbol::Flag => "flag_regular",
            Symbol::Tag => "tag_regular",
            Symbol::Link => "link_regular",
            Symbol::Attach => "attach_regular",
            Symbol::Archive => "archive_regular",
            Symbol::Rename => "rename_regular",
            Symbol::LockClosed => "lock_closed_regular",
            Symbol::LockOpen => "lock_open_regular",
            Symbol::Key => "key_regular",
            Symbol::Eye => "eye_regular",
            Symbol::EyeOff => "eye_off_regular",
            Symbol::Options => "options_regular",
            Symbol::ArrowReply => "arrow_reply_regular",
            Symbol::ArrowForward => "arrow_forward_regular",
            Symbol::ArrowDownload => "arrow_download_regular",
            Symbol::ArrowUpload => "arrow_upload_regular",
            Symbol::Cloud => "cloud_regular",
            Symbol::CloudArrowUp => "cloud_arrow_up_regular",
            Symbol::CloudArrowDown => "cloud_arrow_down_regular",
            Symbol::SelectAll => "select_all_on_regular",
            Symbol::Document => "document_regular",
            Symbol::DocumentAdd => "document_add_regular",
            Symbol::DocumentText => "document_text_regular",
            Symbol::Folder => "folder_regular",
            Symbol::FolderOpen => "folder_open_regular",
            Symbol::FolderAdd => "folder_add_regular",
            Symbol::Image => "image_regular",
            Symbol::ImageMultiple => "image_multiple_regular",
            Symbol::Video => "video_regular",
            Symbol::MusicNote => "music_note_2_regular",
            Symbol::Code => "code_regular",
            Symbol::Database => "database_regular",
            Symbol::Server => "server_regular",
            Symbol::Clipboard => "clipboard_regular",
            Symbol::Notepad => "notepad_regular",
            Symbol::Book => "book_regular",
            Symbol::Library => "library_regular",
            Symbol::Briefcase => "briefcase_regular",
            Symbol::Box => "box_regular",
            Symbol::Info => "info_regular",
            Symbol::Warning => "warning_regular",
            Symbol::ErrorCircle => "error_circle_regular",
            Symbol::CheckmarkCircle => "checkmark_circle_regular",
            Symbol::DismissCircle => "dismiss_circle_regular",
            Symbol::QuestionCircle => "question_circle_regular",
            Symbol::Alert => "alert_regular",
            Symbol::AlertOff => "alert_off_regular",
            Symbol::Shield => "shield_regular",
            Symbol::ShieldCheckmark => "shield_checkmark_regular",
            Symbol::Clock => "clock_regular",
            Symbol::History => "history_regular",
            Symbol::Calendar => "calendar_regular",
            Symbol::Hourglass => "hourglass_regular",
            Symbol::Circle => "circle_regular",
            Symbol::CircleFilled => "circle_filled",
            Symbol::Sparkle => "sparkle_regular",
            Symbol::Lightbulb => "lightbulb_regular",
            Symbol::Bug => "bug_regular",
            Symbol::Wifi => "wifi_1_regular",
            Symbol::Battery => "battery_10_regular",
            Symbol::Flash => "flash_regular",
            Symbol::DataArea => "data_area_regular",
            Symbol::Play => "play_regular",
            Symbol::Pause => "pause_regular",
            Symbol::Stop => "stop_regular",
            Symbol::Previous => "previous_regular",
            Symbol::Next => "next_regular",
            Symbol::Record => "record_regular",
            Symbol::Speaker => "speaker_2_regular",
            Symbol::SpeakerMute => "speaker_mute_regular",
            Symbol::Mic => "mic_regular",
            Symbol::MicOff => "mic_off_regular",
            Symbol::Camera => "camera_regular",
            Symbol::Headphones => "headphones_regular",
            Symbol::FastForward => "fast_forward_regular",
            Symbol::Rewind => "rewind_regular",
            Symbol::Broadcast => "live_regular",
            Symbol::Settings => "settings_regular",
            Symbol::Person => "person_regular",
            Symbol::People => "people_regular",
            Symbol::PersonAdd => "person_add_regular",
            Symbol::PersonCircle => "person_circle_regular",
            Symbol::Color => "color_regular",
            Symbol::PaintBrush => "paint_brush_regular",
            Symbol::DarkTheme => "dark_theme_regular",
            Symbol::WeatherMoon => "weather_moon_regular",
            Symbol::WeatherSunny => "weather_sunny_regular",
            Symbol::Desktop => "desktop_regular",
            Symbol::Laptop => "laptop_regular",
            Symbol::Phone => "phone_regular",
            Symbol::Tablet => "tablet_regular",
            Symbol::Keyboard => "keyboard_regular",
            Symbol::Window => "window_regular",
            Symbol::WindowNew => "window_new_regular",
            Symbol::PlugConnected => "plug_connected_regular",
            Symbol::Power => "power_regular",
            Symbol::Accessibility => "accessibility_regular",
            Symbol::LocalLanguage => "local_language_regular",
            Symbol::TextFont => "text_font_regular",
            Symbol::Wrench => "wrench_regular",
            Symbol::Toolbox => "toolbox_regular",
            Symbol::Mail => "mail_regular",
            Symbol::Chat => "chat_regular",
            Symbol::ChatMultiple => "chat_multiple_regular",
            Symbol::Comment => "comment_regular",
            Symbol::Call => "call_regular",
            Symbol::Megaphone => "megaphone_regular",
            Symbol::TextBold => "text_bold_regular",
            Symbol::TextItalic => "text_italic_regular",
            Symbol::TextUnderline => "text_underline_regular",
            Symbol::TextStrikethrough => "text_strikethrough_regular",
            Symbol::TextAlignLeft => "text_align_left_regular",
            Symbol::TextAlignCenter => "text_align_center_regular",
            Symbol::TextAlignRight => "text_align_right_regular",
            Symbol::TextBulletList => "text_bullet_list_ltr_regular",
            Symbol::TextNumberList => "text_number_list_ltr_regular",
            Symbol::TextHeader1 => "text_header_1_regular",
            Symbol::Table => "table_regular",
            Symbol::TaskListLtr => "task_list_ltr_regular",
            Symbol::TextQuote => "text_quote_regular",
            Symbol::LineHorizontal1 => "line_horizontal_1_regular",
            Symbol::CodeBlock => "code_block_regular",
            Symbol::Branch => "branch_regular",
            Symbol::WindowConsole => "window_console_regular",
            Symbol::PuzzlePiece => "puzzle_piece_regular",
            Symbol::Beaker => "beaker_regular",
            Symbol::Cart => "cart_regular",
            Symbol::Money => "money_regular",
            Symbol::Gift => "gift_regular",
            Symbol::Trophy => "trophy_regular",
            Symbol::Rocket => "rocket_regular",
            Symbol::Emoji => "emoji_regular",
            Symbol::ThumbLike => "thumb_like_regular",
            Symbol::ThumbLikeFilled => "thumb_like_filled",
            Symbol::ThumbDislike => "thumb_dislike_regular",
        }
    }
}
