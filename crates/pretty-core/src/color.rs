//! Colour representation shared with renderers.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse `#rrggbb` or `rrggbb`.
    pub fn parse_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#').unwrap_or(s);
        if s.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Self { r, g, b })
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    pub fn to_array(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }
}

/// A colour reference before theme resolution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorSpec {
    #[default]
    Foreground,
    Background,
    Cursor,
    /// 0-15 are theme palette entries, 16-255 use the xterm cube/ramp.
    Palette(u8),
    Rgb(Rgb),
    /// Default selection foreground/background are resolved by the renderer.
    SelectionForeground,
    SelectionBackground,
}

/// Resolve an alacritty colour, taking runtime OSC 4/10/11 overrides into
/// account. Named foreground/background remain symbolic so theme changes apply
/// without restarting the terminal core.
pub fn resolve(color: Color, colors: &Colors) -> ColorSpec {
    match color {
        Color::Spec(rgb) => ColorSpec::Rgb(Rgb::new(rgb.r, rgb.g, rgb.b)),
        Color::Indexed(index) => {
            if let Some(over) = colors[index as usize] {
                ColorSpec::Rgb(Rgb::new(over.r, over.g, over.b))
            } else {
                ColorSpec::Palette(index)
            }
        }
        Color::Named(named) => match named {
            NamedColor::Foreground | NamedColor::BrightForeground => {
                match colors[NamedColor::Foreground] {
                    Some(rgb) => ColorSpec::Rgb(Rgb::new(rgb.r, rgb.g, rgb.b)),
                    None => ColorSpec::Foreground,
                }
            }
            NamedColor::DimForeground => ColorSpec::Foreground,
            NamedColor::Background => match colors[NamedColor::Background] {
                Some(rgb) => ColorSpec::Rgb(Rgb::new(rgb.r, rgb.g, rgb.b)),
                None => ColorSpec::Background,
            },
            NamedColor::Cursor => match colors[NamedColor::Cursor] {
                Some(rgb) => ColorSpec::Rgb(Rgb::new(rgb.r, rgb.g, rgb.b)),
                None => ColorSpec::Cursor,
            },
            NamedColor::Black
            | NamedColor::Red
            | NamedColor::Green
            | NamedColor::Yellow
            | NamedColor::Blue
            | NamedColor::Magenta
            | NamedColor::Cyan
            | NamedColor::White => named_index(named, colors),
            NamedColor::BrightBlack
            | NamedColor::BrightRed
            | NamedColor::BrightGreen
            | NamedColor::BrightYellow
            | NamedColor::BrightBlue
            | NamedColor::BrightMagenta
            | NamedColor::BrightCyan
            | NamedColor::BrightWhite => named_index(named, colors),
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => named_index(named, colors),
        },
    }
}

fn named_index(named: NamedColor, colors: &Colors) -> ColorSpec {
    // NamedColor discriminants for the first 24 entries map directly to the
    // palette for normal/bright/dim ANSI colours. Dim colours reuse the normal
    // palette entry; the cell's DIM flag performs the actual dimming.
    let idx = match named {
        NamedColor::Black | NamedColor::DimBlack => 0,
        NamedColor::Red | NamedColor::DimRed => 1,
        NamedColor::Green | NamedColor::DimGreen => 2,
        NamedColor::Yellow | NamedColor::DimYellow => 3,
        NamedColor::Blue | NamedColor::DimBlue => 4,
        NamedColor::Magenta | NamedColor::DimMagenta => 5,
        NamedColor::Cyan | NamedColor::DimCyan => 6,
        NamedColor::White | NamedColor::DimWhite => 7,
        NamedColor::BrightBlack => 8,
        NamedColor::BrightRed => 9,
        NamedColor::BrightGreen => 10,
        NamedColor::BrightYellow => 11,
        NamedColor::BrightBlue => 12,
        NamedColor::BrightMagenta => 13,
        NamedColor::BrightCyan => 14,
        NamedColor::BrightWhite => 15,
        _ => 7,
    };
    if let Some(rgb) = colors[named] {
        ColorSpec::Rgb(Rgb::new(rgb.r, rgb.g, rgb.b))
    } else {
        ColorSpec::Palette(idx)
    }
}

/// Convert a 0-255 indexed colour into RGB using the standard xterm palette.
pub fn palette_rgb(index: u8) -> Rgb {
    const ANSI16: [[u8; 3]; 16] = [
        [0x1d, 0x1f, 0x21],
        [0xcc, 0x55, 0x55],
        [0x4c, 0xaf, 0x50],
        [0xd7, 0xa8, 0x43],
        [0x4f, 0x9b, 0xd8],
        [0xc6, 0x78, 0xdd],
        [0x34, 0xb6, 0xb6],
        [0xd8, 0xde, 0xe9],
        [0x60, 0x66, 0x70],
        [0xeb, 0x6f, 0x6f],
        [0x7e, 0xd0, 0x7e],
        [0xf0, 0xc6, 0x74],
        [0x7a, 0xa2, 0xf7],
        [0xd8, 0x9f, 0xe0],
        [0x6d, 0xd3, 0xd3],
        [0xff, 0xff, 0xff],
    ];
    match index {
        0..=15 => {
            let c = ANSI16[index as usize];
            Rgb::new(c[0], c[1], c[2])
        }
        16..=231 => {
            let i = index - 16;
            let r = i / 36;
            let g = (i % 36) / 6;
            let b = i % 6;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Rgb::new(level(r), level(g), level(b))
        }
        232..=255 => {
            let v = 8 + (index - 232) * 10;
            Rgb::new(v, v, v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let c = Rgb::parse_hex("#1a2b3c").unwrap();
        assert_eq!(c, Rgb::new(0x1a, 0x2b, 0x3c));
        assert_eq!(c.to_hex(), "#1a2b3c");
    }

    #[test]
    fn xterm_cube() {
        assert_eq!(palette_rgb(16), Rgb::new(0, 0, 0));
        assert_eq!(palette_rgb(196), Rgb::new(255, 0, 0));
        assert_eq!(palette_rgb(231), Rgb::new(255, 255, 255));
        assert_eq!(palette_rgb(232), Rgb::new(8, 8, 8));
    }
}
