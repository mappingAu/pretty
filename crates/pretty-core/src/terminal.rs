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
            osc52_paste: true,
            cursor_style: CursorStyle {
                shape: VteCursorShape::Block,
                blinking: true,
            },
        }
    }
}

pub struct Terminal {
    term: Term<EventSink>,
    processor: Processor,
    filter: StreamFilter,
    graphics: GraphicsStore,
    events: crossbeam_channel::Sender<TerminalEvent>,
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
            scrolling_history: config.scrollback,
            kitty_keyboard: config.kitty_keyboard,
            osc52,
            default_cursor_style: config.cursor_style,
            ..TermConfig::default()
        };
        let term = Term::new(
            term_config,
            &size,
            EventSink::new(tx.clone()),
        );
        let terminal = Self {
            term,
            processor: Processor::new(),
            filter: StreamFilter::new(),
            graphics: GraphicsStore::new(),
            events: tx,
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
                    let offset = self.term.grid().display_offset();
                    let ev = self.graphics.handle(
                        &payload,
                        cursor.line.0.max(0) as usize,
                        cursor.column.0,
                        offset,
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

    pub fn select_all(&mut self) {
        let top = Line(-(self.term.grid().display_offset() as i32));
        let bottom = Line(top.0 + self.size.rows as i32 - 1);
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
}
