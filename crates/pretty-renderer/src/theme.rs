//! Rendering theme and colour resolution.

use pretty_core::{CellAttributes, ColorSpec, Rgb};

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
        if attrs.hidden {
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
        if is_cursor_block {
            rgb = if is_background { self.cursor } else { self.background };
        }
        let mut alpha = if is_background { self.opacity } else { 1.0 };
        if attrs.dim {
            rgb = Rgb::new(
                (rgb.r as f32 * 0.62) as u8,
                (rgb.g as f32 * 0.62) as u8,
                (rgb.b as f32 * 0.62) as u8,
            );
            if !is_background {
                alpha *= 0.85;
            }
        }
        linear(rgb, alpha)
    }
}
