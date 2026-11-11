//! Thin, renderer-independent wrapper around the terminal state engine.

use std::collections::HashMap;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config as TermConfig, Osc52, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape as VteCursorShape, CursorStyle, Processor};
use log::{debug, warn};

use crate::color::resolve;
use crate::events::{EventSink, TerminalEvent};
use crate::frame::{
    CellAttributes, CursorFrame, CursorShape, Frame, FrameCell, UnderlineStyle,
};
use crate::graphics::GraphicsStore;
use crate::stream::{StreamFilter, StreamItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    pub cols: usize,
    pub rows: usize,
}

impl TermSize {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self { cols: cols.max(2), rows: rows.max(1) }
    }
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Hard upper bound for scrollback history lines.
///
/// A scrollback line costs roughly `columns * cell_size` bytes, so an
/// unbounded value from a config file would be a memory hazard. Both the
/// initial configuration and [`Terminal::set_scrollback`] clamp to this value;
/// the settings UI uses the same 200 000-line maximum.
pub const MAX_SCROLLBACK_LINES: usize = 200_000;

#[derive(Clone, Debug)]
pub struct CoreConfig {
    pub scrollback: usize,
    pub kitty_keyboard: bool,
    pub osc52_copy: bool,
    pub osc52_paste: bool,
    pub cursor_style: CursorStyle,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            scrollback: 10_000,
            kitty_keyboard: true,
            osc52_copy: true,
            // Reading the clipboard on behalf of a process (OSC 52 "paste") is
            // an exfiltration channel for untrusted terminal output; default it
            // off. Alacritty's own default is copy-only for the same reason.
            osc52_paste: false,
            cursor_style: CursorStyle {
                shape: VteCursorShape::Block,
                blinking: true,
            },
        }
    }
}

pub struct Terminal {
    term: Term<EventSink>,
    /// Options last applied to `term`; needed to update a single option without
    /// losing the others (`Term::set_options` takes the whole config).
    term_config: TermConfig,
    processor: Processor,
    filter: StreamFilter,
    graphics: GraphicsStore,
    events: crossbeam_channel::Sender<TerminalEvent>,
    /// The same sink held by the `Term`; used to suppress the spurious title
    /// event that `Term::set_options` emits.
    event_sink: EventSink,
    size: TermSize,
    generation: u64,
    hyperlink_ids: HashMap<String, u32>,
    hyperlink_uris: Vec<String>,
}

impl Terminal {
    pub fn new(
        size: TermSize,
        config: &CoreConfig,
    ) -> (
        Self,
        crossbeam_channel::Receiver<TerminalEvent>,
    ) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let osc52 = match (config.osc52_copy, config.osc52_paste) {
            (true, true) => Osc52::CopyPaste,
            (true, false) => Osc52::OnlyCopy,
            (false, true) => Osc52::OnlyPaste,
            (false, false) => Osc52::Disabled,
        };
        let term_config = TermConfig {
            scrolling_history: config.scrollback.min(MAX_SCROLLBACK_LINES),
            kitty_keyboard: config.kitty_keyboard,
            osc52,
            default_cursor_style: config.cursor_style,
            ..TermConfig::default()
        };
        let event_sink = EventSink::new(tx.clone());
        let term = Term::new(term_config.clone(), &size, event_sink.clone());
        let terminal = Self {
            term,
            term_config,
            processor: Processor::new(),
            filter: StreamFilter::new(),
            graphics: GraphicsStore::new(),
            events: tx,
            event_sink,
            size,
            generation: 0,
            hyperlink_ids: HashMap::new(),
            hyperlink_uris: Vec::new(),
        };
        terminal.send(TerminalEvent::Wakeup);
        (terminal, rx)
    }

    pub fn size(&self) -> TermSize {
        self.size
    }

    pub fn cols(&self) -> usize {
        self.size.cols
    }

    pub fn rows(&self) -> usize {
        self.size.rows
    }

    /// Currently configured scrollback history limit in lines.
    pub fn scrollback(&self) -> usize {
        self.term_config.scrolling_history
    }

    /// Change the maximum scrollback history at runtime.
    ///
    /// `lines` is clamped to [`MAX_SCROLLBACK_LINES`]. Shrinking trims already
    /// retained history immediately (alacritty's `Term::set_options` invokes
    /// `Grid::update_history`); growing takes effect for subsequent output.
    /// All other terminal options (kitty keyboard, OSC 52 policy, cursor
    /// style, escape chars) are preserved. Returns the effective limit.
    pub fn set_scrollback(&mut self, lines: usize) -> usize {
        let lines = lines.min(MAX_SCROLLBACK_LINES);
        if lines == self.term_config.scrolling_history {
            return lines;
        }
        debug!(
            "terminal scrollback limit {} -> {} lines",
            self.term_config.scrolling_history, lines
        );
        let mut config = self.term_config.clone();
        config.scrolling_history = lines;
        // `Term::set_options` emits Title/ResetTitle as a side effect. That is
        // correct for a real option reload, but a scrollback change must not
        // reset the window title.
        self.event_sink.set_suppress_title(true);
        self.term.set_options(config.clone());
        self.event_sink.set_suppress_title(false);
        self.term_config = config;
        self.generation += 1;
        lines
    }

    /// Feed raw PTY bytes into the stream filter and terminal parser.
    pub fn advance(&mut self, bytes: &[u8]) {
        for item in self.filter.feed(bytes) {
            match item {
                StreamItem::Bytes(bytes) => {
                    if !bytes.is_empty() {
                        self.processor.advance(&mut self.term, &bytes);
                    }
                }
                StreamItem::Graphics(payload) => {
                    let cursor = self.term.grid().cursor.point;
                    // The cursor point is a grid line independent of the
                    // current scroll position; the graphics store anchors to
                    // that line and the renderer adds the display offset.
                    let ev = self.graphics.handle(
                        &payload,
                        cursor.line.0.max(0) as usize,
                        cursor.column.0,
                    );
                    if let Some(response) = ev.response {
                        self.send(TerminalEvent::PtyWrite(response));
                    }
                }
                StreamItem::Notification { title, body } => {
                    self.send(TerminalEvent::Notification { title, body });
                }
            }
        }
        self.generation += 1;
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let size = TermSize::new(cols, rows);
        if size == self.size {
            return;
        }
        debug!("resize terminal to {}x{}", size.cols, size.rows);
        self.term.resize(size);
        self.size = size;
        self.generation += 1;
    }

    pub fn scroll(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        self.term.scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
        self.generation += 1;
    }

    pub fn scroll_to_top(&mut self) {
        self.term.scroll_display(alacritty_terminal::grid::Scroll::Top);
        self.generation += 1;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        self.generation += 1;
    }

    pub fn scroll_page_up(&mut self) {
        self.term.scroll_display(alacritty_terminal::grid::Scroll::PageUp);
        self.generation += 1;
    }

    pub fn scroll_page_down(&mut self) {
        self.term.scroll_display(alacritty_terminal::grid::Scroll::PageDown);
        self.generation += 1;
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn history_size(&self) -> usize {
        self.term.total_lines().saturating_sub(self.term.screen_lines())
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn kitty_keyboard_flags(&self) -> u32 {
        (self.term.mode().bits() >> 18) & 0x1f
    }

    pub fn cursor_style(&self) -> CursorStyle {
        self.term.cursor_style()
    }

    pub fn graphics(&self) -> &GraphicsStore {
        &self.graphics
    }

    /// Resolve the hyperlink at a visible cell.
    pub fn hyperlink_at(&self, row: usize, col: usize) -> Option<String> {
        if row >= self.size.rows || col >= self.size.cols {
            return None;
        }
        let line = Line(row as i32 - self.term.grid().display_offset() as i32);
        let point = Point::new(line, Column(col));
        let grid = self.term.grid();
        if point.line < grid.topmost_line() || point.line > grid.bottommost_line() {
            return None;
        }
        grid[point].hyperlink().map(|h| h.uri().to_owned())
    }

    pub fn start_selection(&mut self, row: usize, col: usize) {
        let offset = self.term.grid().display_offset() as i32;
        let point = Point::new(Line(row as i32 - offset), Column(col.min(self.size.cols - 1)));
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, Side::Left));
        self.generation += 1;
    }

    pub fn update_selection(&mut self, row: usize, col: usize) {
        let offset = self.term.grid().display_offset() as i32;
        let point = Point::new(Line(row as i32 - offset), Column(col.min(self.size.cols - 1)));
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, Side::Right);
        } else {
            self.term.selection =
                Some(Selection::new(SelectionType::Simple, point, Side::Right));
        }
        self.generation += 1;
    }

    /// Select the entire terminal contents, including scrollback history.
    ///
    /// The selection endpoints are grid coordinates (negative lines are
    /// history), so the result does not depend on the current scroll position.
    /// Previously the top endpoint was `-display_offset`, which limited the
    /// selection to the visible viewport and silently dropped history lines.
    pub fn select_all(&mut self) {
        let grid = self.term.grid();
        let top = grid.topmost_line();
        let bottom = grid.bottommost_line();
        self.term.selection = Some(Selection::new(
            SelectionType::Simple,
            Point::new(top, Column(0)),
            Side::Left,
        ));
        if let Some(sel) = self.term.selection.as_mut() {
            sel.update(Point::new(bottom, Column(self.size.cols - 1)), Side::Right);
        }
        self.generation += 1;
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    pub fn selection_present(&self) -> bool {
        self.term.selection.is_some()
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
        self.generation += 1;
    }

    /// Full immutable render snapshot.
    pub fn snapshot(&mut self) -> Frame {
        self.generation += 1;
        let generation = self.generation;
        let cols = self.size.cols;
        let rows = self.size.rows;
        let mut cells = vec![FrameCell::default(); cols * rows];
        let mut hyperlink_ids = std::mem::take(&mut self.hyperlink_ids);
        let mut hyperlink_uris = std::mem::take(&mut self.hyperlink_uris);
        // Drop stale links each frame so long-running sessions do not grow.
        hyperlink_ids.clear();
        hyperlink_uris.clear();

        let display_offset = self.term.grid().display_offset();
        let cursor_style = self.term.cursor_style();
        let mut cursor = None;
        {
            let content = self.term.renderable_content();
            let selection = content.selection;
            for indexed in content.display_iter {
                let point = indexed.point;
                let row_i = point.line.0 + display_offset as i32;
                if row_i < 0 || row_i >= rows as i32 {
                    continue;
                }
                let row = row_i as usize;
                let col = point.column.0;
                if col >= cols {
                    continue;
                }
                let src = indexed.cell;
                let mut out = FrameCell::default();
                out.wide = src.flags.contains(Flags::WIDE_CHAR);
                out.wide_spacer = src
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
                if out.wide_spacer {
                    out.c = ' ';
                } else {
                    out.c = if src.c.is_control() { ' ' } else { src.c };
                }
                if let Some(zw) = src.zerowidth() {
                    out.zero_width.extend_from_slice(zw);
                }
                out.fg = resolve(src.fg, content.colors);
                out.bg = resolve(src.bg, content.colors);
                if src.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut out.fg, &mut out.bg);
                }
                let mut attrs = CellAttributes {
                    bold: src.flags.contains(Flags::BOLD),
                    italic: src.flags.contains(Flags::ITALIC),
                    dim: src.flags.contains(Flags::DIM),
                    hidden: src.flags.contains(Flags::HIDDEN),
                    strikeout: src.flags.contains(Flags::STRIKEOUT),
                    underline_color: src
                        .underline_color()
                        .map(|color| resolve(color, content.colors)),
                    underline: if src.flags.contains(Flags::UNDERCURL) {
                        UnderlineStyle::Curly
                    } else if src.flags.contains(Flags::DOUBLE_UNDERLINE) {
                        UnderlineStyle::Double
                    } else if src.flags.contains(Flags::DOTTED_UNDERLINE) {
                        UnderlineStyle::Dotted
                    } else if src.flags.contains(Flags::DASHED_UNDERLINE) {
                        UnderlineStyle::Dashed
                    } else if src.flags.contains(Flags::UNDERLINE) {
                        UnderlineStyle::Single
                    } else {
                        UnderlineStyle::None
                    },
                    ..CellAttributes::default()
                };
                if selection.is_some_and(|range| range.contains(point)) {
                    attrs.selected = true;
                }
                if let Some(link) = src.hyperlink() {
                    let uri = link.uri().to_owned();
                    let id = if let Some(id) = hyperlink_ids.get(&uri) {
                        *id
                    } else {
                        let id = hyperlink_uris.len() as u32;
                        hyperlink_ids.insert(uri.clone(), id);
                        hyperlink_uris.push(uri);
                        id
                    };
                    attrs.hyperlink = Some(id);
                }
                out.attrs = attrs;
                cells[row * cols + col] = out;
            }

            if display_offset == 0 {
                let p = content.cursor.point;
                if p.line.0 >= 0 && p.line.0 < rows as i32 && p.column.0 < cols {
                    let shape = match content.cursor.shape {
                        VteCursorShape::Block => CursorShape::Block,
                        VteCursorShape::Underline => CursorShape::Underline,
                        VteCursorShape::Beam => CursorShape::Beam,
                        VteCursorShape::HollowBlock => CursorShape::HollowBlock,
                        VteCursorShape::Hidden => CursorShape::Hidden,
                    };
                    cursor = Some(CursorFrame {
                        row: p.line.0 as usize,
                        col: p.column.0,
                        shape,
                        blinking: cursor_style.blinking,
                        visible: content.cursor.shape != VteCursorShape::Hidden,
                        off: false,
                    });
                }
            }
        }

        self.hyperlink_ids = hyperlink_ids;
        self.hyperlink_uris = hyperlink_uris;
        let images = self.graphics.frame_images(display_offset);

        Frame {
            cols,
            rows,
            cells,
            cursor,
            display_offset,
            history_size: self.term.total_lines().saturating_sub(self.term.screen_lines()),
            hyperlinks: self.hyperlink_uris.clone(),
            images,
            generation,
        }
    }

    fn send(&self, event: TerminalEvent) {
        if self.events.send(event).is_err() {
            warn!("terminal event receiver disconnected");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use base64::Engine as _;

    use crate::events::ClipboardKind;

    #[test]
    fn default_cursor_is_block() {
        let cfg = CoreConfig::default();
        let (mut term, _rx) = Terminal::new(TermSize::new(20, 5), &cfg);
        let frame = term.snapshot();
        let cursor = frame.cursor.expect("cursor");
        assert_eq!(cursor.shape, CursorShape::Block, "configured cursor style must be used");
    }

    #[test]
    fn kitty_keyboard_push_query_pop() {
        let cfg = CoreConfig::default();
        let (mut term, rx) = Terminal::new(TermSize::new(20, 5), &cfg);
        term.advance(b"\x1b[>31u");
        let _ = term.snapshot();
        let events: Vec<_> = rx.try_iter().collect();
        assert!(term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
        assert!(term.mode().contains(TermMode::REPORT_EVENT_TYPES));
        assert!(term.mode().contains(TermMode::REPORT_ALL_KEYS_AS_ESC));
        term.advance(b"\x1b[?u");
        let query: Vec<_> = rx.try_iter().filter_map(|e| match e {
            TerminalEvent::PtyWrite(s) => Some(s),
            _ => None,
        }).collect();
        assert!(query.iter().any(|s| s == "\x1b[?31u"), "query response: {query:?}");
        term.advance(b"\x1b[<u");
        assert!(!term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
        let _ = events;
    }

    #[test]
    fn decscusr_changes_cursor_shape() {
        let cfg = CoreConfig::default();
        let (mut term, _rx) = Terminal::new(TermSize::new(20, 5), &cfg);
        term.advance(b"\x1b[4 q");
        let frame = term.snapshot();
        assert_eq!(frame.cursor.expect("cursor").shape, CursorShape::Underline);
    }

    #[test]
    fn graphics_anchor_ignores_scroll_position() {
        let cfg = CoreConfig::default();
        let (mut term, _rx) = Terminal::new(TermSize::new(10, 5), &cfg);
        // Fill the screen with history and leave the cursor at the bottom row.
        term.advance("a\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\n".as_bytes());
        term.scroll(3);
        assert_eq!(term.display_offset(), 3);
        let raw = base64::engine::general_purpose::STANDARD.encode([10u8, 20, 30]);
        term.advance(format!("\x1b_Ga=T,f=24,s=1,v=1,i=9;{raw}\x1b\\").as_bytes());
        let frame = term.snapshot();
        assert_eq!(frame.images.len(), 1);
        // The image is anchored to the cursor's grid line (bottom row = 4) and
        // must not have the 3-line scroll offset baked in.
        assert_eq!(frame.images[0].anchor_line, 4);
    }

    #[test]
    fn osc52_read_is_denied_by_default() {
        let cfg = CoreConfig::default();
        let (mut term, rx) = Terminal::new(TermSize::new(20, 5), &cfg);
        term.advance(b"\x1b]52;c;?\x07");
        let events: Vec<_> = rx.try_iter().collect();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, TerminalEvent::ClipboardLoad(..))),
            "OSC 52 clipboard read must be denied by default: {events:?}"
        );
    }

    #[test]
    fn osc52_read_allowed_only_when_configured() {
        let cfg = CoreConfig { osc52_paste: true, ..Default::default() };
        let (mut term, rx) = Terminal::new(TermSize::new(20, 5), &cfg);
        term.advance(b"\x1b]52;c;?\x07");
        let events: Vec<_> = rx.try_iter().collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, TerminalEvent::ClipboardLoad(..))),
            "explicitly enabled OSC 52 paste must reach the app: {events:?}"
        );
    }

    #[test]
    fn set_scrollback_updates_history_limit_and_keeps_terminal_usable() {
        let cfg = CoreConfig::default();
        let (mut term, _rx) = Terminal::new(TermSize::new(10, 5), &cfg);
        assert_eq!(term.scrollback(), 10_000);
        assert_eq!(term.set_scrollback(100), 100);
        assert_eq!(term.scrollback(), 100);
        // Fill history up to the new cap.
        for _ in 0..300 {
            term.advance(b"x\r\n");
        }
        assert_eq!(term.history_size(), 100, "history must be capped at 100 lines");
        // Shrinking trims retained history immediately.
        assert_eq!(term.set_scrollback(10), 10);
        assert_eq!(term.history_size(), 10, "shrink must trim existing history");
        // Growing again takes effect for subsequent output.
        term.set_scrollback(50);
        for _ in 0..100 {
            term.advance(b"y\r\n");
        }
        assert_eq!(term.history_size(), 50);
        // The terminal still parses/render input after option changes.
        term.advance(b"\x1b[2J\x1b[Hok");
        let frame = term.snapshot();
        assert_eq!(frame.cell(0, 0).map(|c| c.c), Some('o'));
        assert_eq!(frame.cell(0, 1).map(|c| c.c), Some('k'));
        assert!(frame.cursor.is_some());
    }

    #[test]
    fn set_scrollback_clamps_to_core_maximum() {
        let (mut term, _rx) = Terminal::new(TermSize::new(10, 5), &CoreConfig::default());
        assert_eq!(term.set_scrollback(usize::MAX), MAX_SCROLLBACK_LINES);
        assert_eq!(term.scrollback(), MAX_SCROLLBACK_LINES);
        assert_eq!(term.set_scrollback(MAX_SCROLLBACK_LINES + 1), MAX_SCROLLBACK_LINES);
    }

    #[test]
    fn initial_scrollback_is_clamped() {
        let cfg = CoreConfig {
            scrollback: usize::MAX,
            ..CoreConfig::default()
        };
        let (mut term, _rx) = Terminal::new(TermSize::new(10, 5), &cfg);
        assert_eq!(term.scrollback(), MAX_SCROLLBACK_LINES);
        term.set_scrollback(4_096);
        assert_eq!(term.scrollback(), 4_096);
    }

    #[test]
    fn set_scrollback_does_not_reset_the_window_title() {
        let (mut term, rx) = Terminal::new(TermSize::new(10, 5), &CoreConfig::default());
        // Drain the initial Wakeup event.
        let _ = rx.try_iter().collect::<Vec<_>>();
        term.set_scrollback(4_321);
        let events: Vec<_> = rx.try_iter().collect();
        assert!(
            !events.iter().any(|event| matches!(
                event,
                TerminalEvent::Title(_) | TerminalEvent::ResetTitle
            )),
            "changing scrollback must not emit a title event: {events:?}"
        );
    }

    #[test]
    fn set_scrollback_preserves_other_terminal_options() {
        let cfg = CoreConfig {
            osc52_paste: true,
            kitty_keyboard: true,
            ..CoreConfig::default()
        };
        let (mut term, _rx) = Terminal::new(TermSize::new(10, 5), &cfg);
        // Push a kitty keyboard mode so a reset would be observable.
        term.advance(b"\x1b[>31u");
        term.set_scrollback(1234);
        assert!(term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
        assert!(term.mode().contains(TermMode::REPORT_ALL_KEYS_AS_ESC));
        // OSC 52 read policy survives the option update.
        term.advance(b"\x1b]52;c;?\x07");
        let events: Vec<_> = _rx.try_iter().collect();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, TerminalEvent::ClipboardLoad(..))),
            "OSC 52 paste policy was lost on set_scrollback: {events:?}"
        );
    }

    #[test]
    fn osc52_write_is_allowed_by_default() {
        let cfg = CoreConfig::default();
        let (mut term, rx) = Terminal::new(TermSize::new(20, 5), &cfg);
        term.advance(b"\x1b]52;c;aGk=\x07"); // base64("hi")
        let events: Vec<_> = rx.try_iter().collect();
        let stored = events.iter().find_map(|e| match e {
            TerminalEvent::ClipboardStore(ClipboardKind::Clipboard, text) => Some(text.clone()),
            _ => None,
        });
        assert_eq!(stored.as_deref(), Some("hi"));
    }
}
