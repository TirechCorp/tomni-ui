//! Terminal do Tomni UI (M17): o widget [`Terminal`], o emulador (VT100, VT220 e xterm) e as
//! conexões com o programa ([`Transport`]).

pub mod cell;
pub mod emulator;
pub mod grid;
pub mod keys;
pub mod links;
mod log;
pub mod mouse;
mod panes;
#[cfg(all(feature = "pty", not(target_os = "android")))]
mod process;
mod quote;
pub mod scheme;
pub mod selection;
#[cfg(feature = "serial")]
pub mod serial;
mod session;
#[cfg(feature = "ssh")]
pub mod ssh;
mod telnet;
pub mod transport;
mod widget;

pub use cell::char_width;
pub use emulator::{
    CursorShape, DEFAULT_SCROLLBACK, Emulator, Hyperlink, Modes, MouseEncoding, MouseTracking,
    Notice, TermProgress,
};
pub use links::{LinkHit, find_link};
pub use log::plain_text;
pub use panes::{
    Broadcast, DuplicatePlace, PANE_BORDER, PANE_DASH, Pane, PaneBorder, PaneDirection, PaneInfo,
    PaneLayout, PaneShortcuts, PaneVisual, PanesActivity, PanesInfo, TerminalPanes, move_session,
};
#[cfg(all(feature = "pty", not(target_os = "android")))]
pub use process::{Process, ProcessBuilder};
pub use quote::{ShellQuote, shell_quote};
pub use scheme::{CAMPBELL, ONE_HALF_LIGHT, Palette, Scheme};
pub use selection::{GridPoint, Selection, SelectionKind};
#[cfg(feature = "serial")]
pub use serial::{Serial, SerialBuilder};
pub use session::{LogMode, OutputTap, Session};
#[cfg(feature = "ssh")]
pub use ssh::{HostKey, KnownHost, Ssh, SshBuilder};
pub use telnet::{Telnet, TelnetBuilder};
pub use tomni_ui_net::Proxy;
pub use transport::{Inbox, Loopback, LoopbackHandle, Status, Transport};
pub use widget::{
    BELL_FLASH, BellStyle, FONT_ZOOM_WHEEL, RichPaste, RichPasteContent, RichPasteSource,
    RightClick, TERMINAL_FONT_MAX, TERMINAL_FONT_MIN, TERMINAL_FONT_SIZE, TERMINAL_MIN_GRID,
    TERMINAL_PADDING, Terminal, TerminalVisual, grid_origin,
};
