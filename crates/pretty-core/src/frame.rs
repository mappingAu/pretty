//! Renderer-facing immutable snapshots of a terminal pane.
//!
//! The renderer crate only sees these types; it never depends on
//! `alacritty_terminal`.

use std::sync::Arc;

use crate::color::ColorSpec;
use crate::graphics::FrameImage;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellAttributes {
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    pub hidden: bool,
    pub strikeout: bool,
    pub underline: UnderlineStyle,
    pub underline_color: Option<ColorSpec>,
    pub hyperlink: Option<u32>,
    pub selected: bool,
}

#[derive(Clone, Debug)]
pub struct FrameCell {
    /// Primary character. Spaces are used for empty cells.
    pub c: char,
    /// Combining / zero-width characters that follow `c`.
    pub zero_width: Vec<char>,
    /// The cell is the second half of a full-width character and must not be
    /// rendered independently.
    pub wide_spacer: bool,
    /// This cell holds the leading half of a full-width character.
    pub wide: bool,
    pub fg: ColorSpec,
    pub bg: ColorSpec,
    pub attrs: CellAttributes,
}

impl Default for FrameCell {
    fn default() -> Self {
        Self {
            c: ' ',
            zero_width: Vec::new(),
            wide_spacer: false,
            wide: false,
            fg: ColorSpec::Foreground,
            bg: ColorSpec::Background,
            attrs: CellAttributes::default(),
        }
    }
}

impl FrameCell {
    /// Text that must be shaped for this cell.
    pub fn text(&self) -> String {
        let mut s = String::with_capacity(4 + self.zero_width.len());
        s.push(self.c);
        s.extend(self.zero_width.iter());
        s
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
    Hidden,
}

#[derive(Clone, Copy, Debug)]
pub struct CursorFrame {
    pub row: usize,
    pub col: usize,
    pub shape: CursorShape,
    pub blinking: bool,
    /// A blinking cursor may be in the "off" phase; this is applied by the app.
    pub visible: bool,
    /// Blink off phase: draw a dim outline instead of removing the cursor.
    /// This keeps the cursor location visible without the full block inversion.
    pub off: bool,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<FrameCell>,
    pub cursor: Option<CursorFrame>,
    pub display_offset: usize,
    pub history_size: usize,
    pub hyperlinks: Vec<String>,
    pub images: Vec<FrameImage>,
    pub generation: u64,
}

impl Frame {
    pub fn cell(&self, row: usize, col: usize) -> Option<&FrameCell> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        self.cells.get(row * self.cols + col)
    }

    /// URI of a terminal hyperlink at a visible cell, if any.
    pub fn hyperlink_at(&self, row: usize, col: usize) -> Option<&str> {
        let cell = self.cell(row, col)?;
        let id = cell.attrs.hyperlink?;
        self.hyperlinks.get(id as usize).map(String::as_str)
    }

    pub fn selection_text_from_cells(&self) -> String {
        let mut lines: Vec<String> = Vec::with_capacity(self.rows);
        for row in 0..self.rows {
            let mut last_non_space = 0usize;
            let mut line = String::new();
            for col in 0..self.cols {
                let Some(cell) = self.cell(row, col) else { break };
                if cell.wide_spacer {
                    // The second half of a wide character is not text: the
                    // leading cell already carries the full character. Adding
                    // a space here used to insert a spurious blank after every
                    // CJK/emoji character in a copied selection.
                    continue;
                }
                if cell.attrs.selected {
                    line.push(cell.c);
                    line.extend(cell.zero_width.iter());
                    if cell.c != ' ' {
                        last_non_space = line.len();
                    }
                } else {
                    line.push(' ');
                }
            }
            line.truncate(last_non_space);
            lines.push(line);
        }
        // Rows outside the selection are empty; keeping them would prefix the
        // copied text with blank lines whose count depends on the selection
        // position (and on scrollback state).
        while lines.first().is_some_and(|line| line.is_empty()) {
            lines.remove(0);
        }
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }
}

/// Image content shared with the renderer without copying the pixel buffer.
#[derive(Clone)]
pub struct SharedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl std::fmt::Debug for SharedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba_len", &self.rgba.len())
            .finish()
    }
}
