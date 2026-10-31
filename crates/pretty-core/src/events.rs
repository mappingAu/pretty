//! Events emitted by the terminal parser and delivered to the application.

use std::fmt;
use std::process::ExitStatus;
use std::sync::Arc;

use alacritty_terminal::event::{Event as VteEvent, EventListener, WindowSize};
use alacritty_terminal::term::ClipboardType;
use alacritty_terminal::vte::ansi::Rgb;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardKind {
    Clipboard,
    Selection,
}

impl From<ClipboardType> for ClipboardKind {
    fn from(value: ClipboardType) -> Self {
        match value {
            ClipboardType::Clipboard => Self::Clipboard,
            ClipboardType::Selection => Self::Selection,
        }
    }
}

pub type ClipboardFormatter = Arc<dyn Fn(&str) -> String + Send + Sync + 'static>;
pub type ColorFormatter = Arc<dyn Fn(Rgb) -> String + Send + Sync + 'static>;
pub type SizeFormatter = Arc<dyn Fn(u16, u16, u16, u16) -> String + Send + Sync + 'static>;

#[derive(Clone)]
pub enum TerminalEvent {
    Title(String),
    ResetTitle,
    Bell,
    ClipboardStore(ClipboardKind, String),
    ClipboardLoad(ClipboardKind, ClipboardFormatter),
    ColorRequest(usize, ColorFormatter),
    /// Response bytes that must be written to the PTY.
    PtyWrite(String),
    TextAreaSizeRequest(SizeFormatter),
    CursorBlinkingChange,
    Wakeup,
    Exit,
    ChildExit(ExitStatus),
    /// An OSC 9 or OSC 777 notification.
    Notification { title: String, body: String },
}

impl fmt::Debug for TerminalEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Title(t) => f.debug_tuple("Title").field(t).finish(),
            Self::ResetTitle => f.write_str("ResetTitle"),
            Self::Bell => f.write_str("Bell"),
            Self::ClipboardStore(kind, text) => f
                .debug_tuple("ClipboardStore")
                .field(kind)
                .field(&format_args!("<{} bytes>", text.len()))
                .finish(),
            Self::ClipboardLoad(kind, _) => f.debug_tuple("ClipboardLoad").field(kind).finish(),
            Self::ColorRequest(index, _) => f.debug_tuple("ColorRequest").field(index).finish(),
            Self::PtyWrite(text) => f.debug_tuple("PtyWrite").field(text).finish(),
            Self::TextAreaSizeRequest(_) => f.write_str("TextAreaSizeRequest"),
            Self::CursorBlinkingChange => f.write_str("CursorBlinkingChange"),
            Self::Wakeup => f.write_str("Wakeup"),
            Self::Exit => f.write_str("Exit"),
            Self::ChildExit(status) => f.debug_tuple("ChildExit").field(status).finish(),
            Self::Notification { title, body } => f
                .debug_struct("Notification")
                .field("title", title)
                .field("body", body)
                .finish(),
        }
    }
}

/// Adapter between `alacritty_terminal` and Pretty's event channel.
#[derive(Clone)]
pub struct EventSink {
    sender: crossbeam_channel::Sender<TerminalEvent>,
}

impl EventSink {
    pub fn new(sender: crossbeam_channel::Sender<TerminalEvent>) -> Self {
        Self { sender }
    }
}

impl EventListener for EventSink {
    fn send_event(&self, event: VteEvent) {
        let event = match event {
            VteEvent::Title(title) => TerminalEvent::Title(title),
            VteEvent::ResetTitle => TerminalEvent::ResetTitle,
            VteEvent::Bell => TerminalEvent::Bell,
            VteEvent::ClipboardStore(kind, text) => {
                TerminalEvent::ClipboardStore(kind.into(), text)
            }
            VteEvent::ClipboardLoad(kind, formatter) => {
                TerminalEvent::ClipboardLoad(kind.into(), formatter)
            }
            VteEvent::ColorRequest(index, formatter) => {
                TerminalEvent::ColorRequest(index, formatter)
            }
            VteEvent::PtyWrite(text) => TerminalEvent::PtyWrite(text),
            VteEvent::TextAreaSizeRequest(formatter) => {
                let wrapped: SizeFormatter =
                    Arc::new(move |cols, rows, cell_w, cell_h| {
                        formatter(WindowSize {
                            num_lines: rows,
                            num_cols: cols,
                            cell_width: cell_w,
                            cell_height: cell_h,
                        })
                    });
                TerminalEvent::TextAreaSizeRequest(wrapped)
            }
            VteEvent::CursorBlinkingChange => TerminalEvent::CursorBlinkingChange,
            VteEvent::Wakeup => TerminalEvent::Wakeup,
            VteEvent::Exit => TerminalEvent::Exit,
            VteEvent::ChildExit(status) => TerminalEvent::ChildExit(status),
            VteEvent::MouseCursorDirty => TerminalEvent::Wakeup,
        };
        let _ = self.sender.send(event);
    }
}
