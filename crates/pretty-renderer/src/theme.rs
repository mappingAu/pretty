//! Rendering theme and colour resolution.

use pretty_core::{CellAttributes, ColorSpec, Rgb};

/// Scrollbar visibility policy (kitty `scrollbar`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScrollbarMode {
    /// Always reserve and draw the scrollbar track.
    Always,
    /// Show the scrollbar only while history is scrolled (kitty default).
    #[default]
    Scrolled,
    /// Never draw a scrollbar.
    Never,
}

impl ScrollbarMode {
    pub fn from_config(value: &str) -> Self {
        match value {
            "always" => Self::Always,
            "never" => Self::Never,
            _ => Self::Scrolled,
        }
    }

    /// Whether the scrollbar should be drawn for the given scroll offset.
    pub fn visible(self, display_offset: usize) -> bool {
        match self {
            Self::Always => true,
            Self::Scrolled => display_offset > 0,
            Self::Never => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub selection_bg: Rgb,
    pub selection_fg: Rgb,
    pub accent: Rgb,
    pub panel: Rgb,
    pub border: Rgb,
    pub palette: [Rgb; 16],
    pub opacity: f32,
    pub font_family: String,
    pub font_size: f32,
    pub font_features: Vec<String>,
    pub ligatures: bool,
    pub line_height: f32,
    pub padding: f32,
    pub gap: f32,
    pub cursor_blink: bool,
    /// Cursor opacity (kitty `cursor_opacity`), 0.0-1.0.
    pub cursor_opacity: f32,
    /// Text colour under a block cursor; `None` uses the background colour.
    pub cursor_text: Option<Rgb>,
    /// How strongly dim text is faded toward the background (kitty `dim_opacity`).
    pub dim_opacity: f32,
    /// Border colour of the focused pane; `None` uses the neutral border.
    pub active_border: Option<Rgb>,
    /// Border colour of unfocused panes; `None` uses the neutral border.
    pub inactive_border: Option<Rgb>,
    pub scrollbar: ScrollbarMode,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Rgb::new(0x0d, 0x11, 0x17),
            foreground: Rgb::new(0xd8, 0xde, 0xe9),
            cursor: Rgb::new(0x88, 0xc0, 0xd0),
            selection_bg: Rgb::new(0x33, 0x4b, 0x67),
            selection_fg: Rgb::new(0xec, 0xef, 0xf4),
            accent: Rgb::new(0x7a, 0xa2, 0xf7),
            panel: Rgb::new(0x16, 0x1c, 0x26),
            border: Rgb::new(0x2a, 0x33, 0x44),
            palette: [
                Rgb::new(0x1d, 0x1f, 0x21),
                Rgb::new(0xcc, 0x55, 0x55),
                Rgb::new(0x4c, 0xaf, 0x50),
                Rgb::new(0xd7, 0xa8, 0x43),
                Rgb::new(0x4f, 0x9b, 0xd8),
                Rgb::new(0xc6, 0x78, 0xdd),
                Rgb::new(0x34, 0xb6, 0xb6),
                Rgb::new(0xd8, 0xde, 0xe9),
                Rgb::new(0x60, 0x66, 0x70),
                Rgb::new(0xeb, 0x6f, 0x6f),
                Rgb::new(0x7e, 0xd0, 0x7e),
                Rgb::new(0xf0, 0xc6, 0x74),
                Rgb::new(0x7a, 0xa2, 0xf7),
                Rgb::new(0xd8, 0x9f, 0xe0),
                Rgb::new(0x6d, 0xd3, 0xd3),
                Rgb::new(0xff, 0xff, 0xff),
            ],
            opacity: 0.9,
            font_family: "JetBrains Mono".into(),
            font_size: 15.0,
            font_features: vec!["liga".into(), "calt".into()],
            ligatures: true,
            line_height: 1.18,
            padding: 10.0,
            gap: 4.0,
            cursor_blink: true,
            cursor_opacity: 1.0,
            cursor_text: None,
            dim_opacity: 0.5,
            active_border: None,
            inactive_border: None,
            scrollbar: ScrollbarMode::Scrolled,
        }
    }
}

fn srgb_channel(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear(rgb: Rgb, alpha: f32) -> [f32; 4] {
    [
        srgb_channel(rgb.r),
        srgb_channel(rgb.g),
        srgb_channel(rgb.b),
        alpha,
    ]
}

pub fn xterm_rgb(index: u8) -> Rgb {
    pretty_core::color::palette_rgb(index)
}

impl Theme {
    pub fn background_linear(&self) -> [f32; 4] {
        linear(self.background, self.opacity)
    }

    pub fn foreground_linear(&self) -> [f32; 4] {
        linear(self.foreground, 1.0)
    }

    pub fn resolve(
        &self,
        spec: ColorSpec,
        attrs: &CellAttributes,
        is_cursor_block: bool,
        is_background: bool,
    ) -> [f32; 4] {
        // Hidden text is transparent, but an explicit cell background must
        // still be painted (SGR 8 only hides the glyph, not the cell).
        if attrs.hidden && !is_background {
            return [0.0, 0.0, 0.0, 0.0];
        }
        let mut rgb = match spec {
            ColorSpec::Foreground => self.foreground,
            ColorSpec::Background => self.background,
            ColorSpec::Cursor => self.cursor,
            ColorSpec::SelectionForeground => self.selection_fg,
            ColorSpec::SelectionBackground => self.selection_bg,
            ColorSpec::Rgb(rgb) => rgb,
            ColorSpec::Palette(i) if (i as usize) < 16 => self.palette[i as usize],
            ColorSpec::Palette(i) => xterm_rgb(i),
        };
        if attrs.selected {
            rgb = if is_background { self.selection_bg } else { self.selection_fg };
        }
        let mut cursor_alpha = 1.0;
        if is_cursor_block {
            rgb = if is_background {
                self.cursor
            } else {
                self.cursor_text.unwrap_or(self.background)
            };
            if is_background {
                // `cursor_opacity` fades the block itself, never the text.
                cursor_alpha = self.cursor_opacity.clamp(0.0, 1.0);
            }
        }
        let mut alpha = if is_background { self.opacity } else { 1.0 } * cursor_alpha;
        if attrs.dim {
            // Blend toward the terminal background like kitty's dim_opacity
            // instead of dropping alpha, which would leak the desktop through.
            let f = self.dim_opacity.clamp(0.0, 1.0);
            let mix = |c: u8, b: u8| (b as f32 + (c as f32 - b as f32) * f).round() as u8;
            rgb = Rgb::new(
                mix(rgb.r, self.background.r),
                mix(rgb.g, self.background.g),
                mix(rgb.b, self.background.b),
            );
        }
        linear(rgb, alpha)
    }
}
