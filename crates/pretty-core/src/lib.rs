//! Pretty terminal core.
//!
//! This crate owns terminal state, PTY plumbing and the kitty protocol
//! extensions. It deliberately contains no GPU, windowing or UI framework
//! dependencies. Renderers consume the snapshots defined in [`frame`].

pub mod color;
pub mod events;
pub mod frame;
pub mod graphics;
pub mod input;
pub mod layout;
pub mod notify;
pub mod pty;
pub mod stream;
pub mod terminal;

pub use color::{ColorSpec, Rgb};
pub use events::{ClipboardKind, TerminalEvent};
pub use frame::{
    CellAttributes, CursorFrame, CursorShape, Frame, FrameCell, UnderlineStyle,
};
pub use graphics::{FrameImage, GraphicsStore};
pub use input::{
    encode_key, encode_mouse, encode_paste, Key, KeyEventKind, KeyInput, Modifiers,
    MouseButton, MouseEventKind,
};
pub use layout::{Axis, Layout, PaneId, Rect};
pub use pty::{PtyExitStatus, PtySession};
pub use terminal::{CoreConfig, Terminal, TermSize, MAX_SCROLLBACK_LINES};
/// Re-export needed to answer DA/colour queries from the application.
pub use alacritty_terminal::vte::ansi::Rgb as VteRgb;

/// Errors shared by the terminal core.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("pty error: {0}")]
    Pty(String),
    #[error("terminal I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid terminal dimensions: {0}x{1}")]
    InvalidSize(usize, usize),
    #[error("configuration error: {0}")]
    Config(String),
}
/// Terminal mode flags from the state engine, re-exported for input policy.
pub use alacritty_terminal::term::TermMode;
/// Cursor style type used in the core configuration.
pub use alacritty_terminal::vte::ansi::{CursorShape as CoreCursorShape, CursorStyle};
