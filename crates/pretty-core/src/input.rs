//! Keyboard and mouse protocol encoders.
//!
//! Keyboard handling follows the kitty progressive enhancement flags exposed
//! by alacritty's `TermMode`, plus legacy xterm behaviour when no flags are
//! active. The encoder is pure and exhaustively unit-tested.

use alacritty_terminal::term::TermMode;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_key: bool,
}

impl Modifiers {
    pub const NONE: Self = Self { shift: false, ctrl: false, alt: false, super_key: false };

    pub fn is_empty(self) -> bool {
        !self.shift && !self.ctrl && !self.alt && !self.super_key
    }

    fn legacy_bits(self) -> u8 {
        (self.shift as u8) | ((self.alt as u8) << 1) | ((self.ctrl as u8) << 2) | ((self.super_key as u8) << 3)
    }

    fn kitty_bits(self) -> u8 {
        (self.shift as u8) | ((self.alt as u8) << 1) | ((self.ctrl as u8) << 2) | ((self.super_key as u8) << 3)
    }

    fn legacy_param(self) -> u8 {
        1 + self.legacy_bits()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventKind {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Tab,
    Backspace,
    Escape,
    Insert,
    Delete,
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
    F(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Named(NamedKey),
}

#[derive(Clone, Debug)]
pub struct KeyInput {
    pub key: Key,
    /// Text produced by the key according to the active keyboard layout and
    /// modifiers, if any.
    pub text: Option<String>,
    /// The unshifted key character, used by the kitty "report alternate keys"
    /// flag. For non-ASCII layouts this is the key without Shift.
    pub base_layout: Option<char>,
    pub modifiers: Modifiers,
    pub kind: KeyEventKind,
}

impl KeyInput {
    pub fn new(key: Key) -> Self {
        Self {
            key,
            text: None,
            base_layout: None,
            modifiers: Modifiers::NONE,
            kind: KeyEventKind::Press,
        }
    }

    pub fn char(c: char) -> Self {
        Self { text: Some(c.to_string()), ..Self::new(Key::Char(c)) }
    }

    pub fn named(named: NamedKey) -> Self {
        Self::new(Key::Named(named))
    }
}

fn kitty_flags(mode: TermMode) -> u32 {
    mode.bits() >> 18 & 0x1f
}

fn kitty_functional_code(key: NamedKey) -> u32 {
    match key {
        NamedKey::Insert => 57348,
        NamedKey::Delete => 57349,
        NamedKey::Left => 57350,
        NamedKey::Right => 57351,
        NamedKey::Up => 57352,
        NamedKey::Down => 57353,
        NamedKey::PageUp => 57354,
        NamedKey::PageDown => 57355,
        NamedKey::Home => 57356,
        NamedKey::End => 57357,
        NamedKey::CapsLock => 57358,
        NamedKey::ScrollLock => 57359,
        NamedKey::NumLock => 57360,
        NamedKey::PrintScreen => 57361,
        NamedKey::Pause => 57362,
        NamedKey::Menu => 57363,
        NamedKey::F(n) => 57363 + n as u32,
        NamedKey::Escape => 27,
        NamedKey::Enter => 13,
        NamedKey::Tab => 9,
        NamedKey::Backspace => 127,
    }
}

fn key_unicode(key: Key) -> Option<u32> {
    match key {
        Key::Char(c) => Some(c as u32),
        Key::Named(n) => Some(kitty_functional_code(n)),
    }
}

fn event_type(kind: KeyEventKind) -> Option<u8> {
    match kind {
        KeyEventKind::Press => Some(1),
        KeyEventKind::Repeat => Some(2),
        KeyEventKind::Release => Some(3),
    }
}

/// Encode a single key event according to the active terminal modes.
pub fn encode_key(mode: TermMode, event: &KeyInput) -> Vec<u8> {
    let flags = kitty_flags(mode);
    if event.kind == KeyEventKind::Release && flags & 2 == 0 {
        return Vec::new();
    }
    let kitty_active = flags != 0;
    let associated_text = flags & 16 != 0;
    let event_types = flags & 2 != 0;
    let all_keys = flags & 8 != 0;
    let alternate = flags & 4 != 0;

    let has_text = event.text.as_deref().is_some_and(|t| !t.is_empty());
    let mutating = event.modifiers.ctrl || event.modifiers.alt || event.modifiers.super_key;

    if kitty_active {
        // kitty's disambiguate rules keep Enter, Tab and Backspace on their
        // legacy C0 encodings (and suppress their release events unless
        // "report all keys" is active) so that a user can still type `reset`
        // after an application that enabled the protocol crashed without
        // restoring it. The old encoder emitted `CSI 13;1u` etc. for these
        // keys when only disambiguate was requested.
        if !all_keys {
            if let Key::Named(
                named @ (NamedKey::Enter | NamedKey::Tab | NamedKey::Backspace),
            ) = event.key
            {
                if event.kind == KeyEventKind::Release {
                    return Vec::new();
                }
                return encode_legacy_c0(named, event.modifiers);
            }
        }
        // Plain printable text without ctrl/alt/super remains ordinary text
        // unless the app requested every key as CSI-u (or associated text,
        // which is only defined together with report-all-keys). This keeps
        // shell input and IME text working. Report-event-types on its own does
        // not turn text keys into escape codes and does not report a release
        // for them; kitty deliberately has no release event for text keys in
        // that mode.
        let text_key = !mutating && !all_keys && !associated_text;
        if text_key {
            if event.kind == KeyEventKind::Release {
                return Vec::new();
            }
            if let Some(text) = &event.text {
                if !text.is_empty() && !text.chars().any(char::is_control) {
                    return text.as_bytes().to_vec();
                }
            }
            if let Key::Char(c) = event.key {
                let mut buf = [0u8; 4];
                return c.encode_utf8(&mut buf).as_bytes().to_vec();
            }
            // Named functional keys fall through to the escape encoder.
        }
        return encode_kitty(event, event_types, alternate, associated_text, has_text);
    }

    encode_legacy(mode, event)
}

fn encode_kitty(
    event: &KeyInput,
    event_types: bool,
    alternate: bool,
    associated_text: bool,
    has_text: bool,
) -> Vec<u8> {
    let Some(raw_code) = key_unicode(event.key) else {
        return Vec::new();
    };
    // kitty requires the unicode-key-code to be the *unshifted* key ("the
    // codepoint used is always the lower-case (or more technically,
    // un-shifted) version of the key"). The logical key reported by the
    // windowing system is the shifted one for Shift+<key>, so recover the
    // unshifted scalar from the base-layout key (winit's
    // key-without-modifiers) or, for ASCII letters, by lower-casing. The
    // shifted scalar is reported as the first alternate sub-field.
    let mut code = raw_code;
    let mut shifted = None;
    if let Key::Char(c) = event.key {
        if event.modifiers.shift {
            let unshifted = event
                .base_layout
                .filter(|base| *base != c)
                .unwrap_or_else(|| {
                    if c.is_ascii_alphabetic() {
                        c.to_ascii_lowercase()
                    } else {
                        c
                    }
                });
            if unshifted != c {
                code = unshifted as u32;
                shifted = Some(c as u32);
            }
        }
    }
    // Alternate-key reporting: key[:shifted[:base]].
    let mut key_param = code.to_string();
    if alternate {
        let mut parts = vec![code.to_string()];
        match shifted {
            Some(shifted_code) => {
                parts.push(shifted_code.to_string());
                if let Some(base) = event.base_layout {
                    let base_code = base as u32;
                    if base_code != code {
                        parts.push(base_code.to_string());
                    }
                }
            }
            None => {
                if let Some(base) = event.base_layout {
                    let base_code = base as u32;
                    if base_code != code {
                        // No shifted key, only a base layout key: use an empty
                        // sub-field so the base key is not misread as shifted.
                        parts.push(String::new());
                        parts.push(base_code.to_string());
                    }
                }
            }
        }
        key_param = parts.join(":");
    }
    let mods = 1 + event.modifiers.kitty_bits() as u32;
    let mut modifier = mods.to_string();
    if event_types {
        let t = event_type(event.kind).unwrap_or(1);
        modifier.push(':');
        modifier.push_str(&t.to_string());
    }
    let mut seq = format!("\x1b[{key_param};{modifier}");
    // The modifier parameter can be omitted only when it is exactly 1 and
    // there is no event/text parameter. Keeping it explicit is valid and
    // simpler for applications.
    if associated_text && has_text && event.kind != KeyEventKind::Release {
        let text = event.text.as_deref().unwrap_or("");
        let cps: Vec<String> = text
            .chars()
            .filter(|c| !c.is_control())
            .map(|c| (c as u32).to_string())
            .collect();
        if !cps.is_empty() {
            seq.push(';');
            seq.push_str(&cps.join(":"));
        }
    }
    seq.push('u');
    seq.into_bytes()
}

fn encode_legacy(mode: TermMode, event: &KeyInput) -> Vec<u8> {
    let m = event.modifiers;
    match event.key {
        Key::Char(c) => {
            if event.kind == KeyEventKind::Release {
                return Vec::new();
            }
            // Ctrl+letter or Ctrl+symbol maps to a C0 control character.
            if m.ctrl {
                if let Some(ctrl) = control_char(c) {
                    let mut out = Vec::new();
                    if m.alt {
                        out.push(0x1b);
                    }
                    out.push(ctrl);
                    return out;
                }
            }
            if let Some(text) = &event.text {
                if !text.is_empty() {
                    let mut out = Vec::new();
                    if m.alt {
                        out.push(0x1b);
                    }
                    out.extend_from_slice(text.as_bytes());
                    return out;
                }
            }
            let mut out = Vec::new();
            if m.alt {
                out.push(0x1b);
            }
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            out
        }
        Key::Named(named) => {
            if event.kind == KeyEventKind::Release {
                return Vec::new();
            }
            encode_legacy_named(mode, named, m)
        }
    }
}

fn control_char(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        '@' | ' ' => Some(0),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

/// Legacy C0 encodings for Enter, Tab and Backspace.
///
/// kitty's compatibility table (and xterm) differ from the naive encoding for
/// ctrl/alt combinations: ctrl+Backspace is `0x08`, alt+Tab is `ESC TAB` and
/// shift+alt+Tab is `ESC CSI Z`. These keys deliberately stay legacy even when
/// the disambiguate flag is active (unless report-all-keys is also requested).
fn encode_legacy_c0(key: NamedKey, m: Modifiers) -> Vec<u8> {
    let mut out = Vec::new();
    match key {
        NamedKey::Enter => {
            if m.alt {
                out.push(0x1b);
            }
            out.push(b'\r');
        }
        NamedKey::Tab => {
            if m.alt {
                out.push(0x1b);
            }
            if m.shift {
                out.extend_from_slice(b"\x1b[Z");
            } else {
                out.push(b'\t');
            }
        }
        NamedKey::Backspace => {
            if m.alt {
                out.push(0x1b);
            }
            out.push(if m.ctrl { 0x08 } else { 0x7f });
        }
        _ => {}
    }
    out
}

fn encode_legacy_named(mode: TermMode, key: NamedKey, m: Modifiers) -> Vec<u8> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let mods = m.legacy_param();
    let numbered = |code: u8| -> Vec<u8> {
        if m.legacy_bits() == 0 {
            format!("\x1b[{code}~").into_bytes()
        } else {
            format!("\x1b[{code};{mods}~").into_bytes()
        }
    };
    match key {
        NamedKey::Enter | NamedKey::Tab | NamedKey::Backspace => {
            encode_legacy_c0(key, m)
        }
        NamedKey::Escape => vec![0x1b],
        NamedKey::Insert => numbered(2),
        NamedKey::Delete => numbered(3),
        NamedKey::PageUp => numbered(5),
        NamedKey::PageDown => numbered(6),
        NamedKey::Home => {
            if app_cursor && m.legacy_bits() == 0 {
                b"\x1bOH".to_vec()
            } else if m.legacy_bits() == 0 {
                b"\x1b[H".to_vec()
            } else {
                format!("\x1b[1;{mods}H").into_bytes()
            }
        }
        NamedKey::End => {
            if app_cursor && m.legacy_bits() == 0 {
                b"\x1bOF".to_vec()
            } else if m.legacy_bits() == 0 {
                b"\x1b[F".to_vec()
            } else {
                format!("\x1b[1;{mods}F").into_bytes()
            }
        }
        NamedKey::Left => arrow(app_cursor, mods, m, 'D'),
        NamedKey::Right => arrow(app_cursor, mods, m, 'C'),
        NamedKey::Up => arrow(app_cursor, mods, m, 'A'),
        NamedKey::Down => arrow(app_cursor, mods, m, 'B'),
        NamedKey::F(n) => function_key(n, mods, m),
        NamedKey::CapsLock
        | NamedKey::ScrollLock
        | NamedKey::NumLock
        | NamedKey::PrintScreen
        | NamedKey::Pause
        | NamedKey::Menu => Vec::new(),
    }
}

fn arrow(app_cursor: bool, _mods: u8, m: Modifiers, final_char: char) -> Vec<u8> {
    if m.legacy_bits() == 0 {
        if app_cursor {
            format!("\x1bO{final_char}").into_bytes()
        } else {
            format!("\x1b[{final_char}").into_bytes()
        }
    } else {
        format!("\x1b[1;{}{final_char}", m.legacy_param()).into_bytes()
    }
}

fn function_key(n: u8, _mods: u8, m: Modifiers) -> Vec<u8> {
    let suffix = match n {
        1 => 'P',
        2 => 'Q',
        3 => 'R',
        4 => 'S',
        5 => '~',
        6 => '~',
        7 => '~',
        8 => '~',
        9 => '~',
        10 => '~',
        11 => '~',
        12 => '~',
        13 => '~',
        14 => '~',
        15 => '~',
        16 => '~',
        17 => '~',
        18 => '~',
        19 => '~',
        20 => '~',
        _ => return Vec::new(),
    };
    if (1..=4).contains(&n) {
        if m.legacy_bits() == 0 {
            return format!("\x1bO{suffix}").into_bytes();
        }
        return format!("\x1b[1;{}{suffix}", m.legacy_param()).into_bytes();
    }
    let code = match n {
        5 => 15,
        6 => 17,
        7 => 18,
        8 => 19,
        9 => 20,
        10 => 21,
        11 => 23,
        12 => 24,
        13 => 25,
        14 => 26,
        15 => 28,
        16 => 29,
        17 => 31,
        18 => 32,
        19 => 33,
        20 => 34,
        _ => return Vec::new(),
    };
    if m.legacy_bits() == 0 {
        format!("\x1b[{code}~").into_bytes()
    } else {
        format!("\x1b[{code};{}~", m.legacy_param()).into_bytes()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
    Back,
    Forward,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEventKind {
    Press,
    Release,
    Motion,
}

/// Encode a mouse event for the terminal's active tracking mode.
///
/// Returns `None` when mouse reporting is disabled or the event is filtered by
/// the current tracking mode (for example motion in 1000 mode).
pub fn encode_mouse(
    mode: TermMode,
    button: MouseButton,
    kind: MouseEventKind,
    col: usize,
    row: usize,
    m: Modifiers,
) -> Option<Vec<u8>> {
    let any_motion = mode.contains(TermMode::MOUSE_MOTION);
    let drag_motion = mode.contains(TermMode::MOUSE_DRAG);
    let click = mode.contains(TermMode::MOUSE_REPORT_CLICK);
    if !(any_motion || drag_motion || click) {
        return None;
    }
    if kind == MouseEventKind::Motion {
        if !any_motion && !(drag_motion && button != MouseButton::None) {
            return None;
        }
    }
    let mut cb = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
        MouseButton::WheelLeft => 66,
        MouseButton::WheelRight => 67,
        MouseButton::Back => 128,
        MouseButton::Forward => 129,
        MouseButton::None => 3,
    };
    if kind == MouseEventKind::Motion {
        cb += 32;
    }
    // SGR encodes release with the original button and a final 'm'. Legacy
    // X10 encoding uses button 3 for releases.
    if kind == MouseEventKind::Release
        && !matches!(button, MouseButton::WheelUp | MouseButton::WheelDown)
        && !mode.contains(TermMode::SGR_MOUSE)
    {
        cb = 3;
    }
    if m.shift {
        cb += 4;
    }
    if m.alt {
        cb += 8;
    }
    if m.ctrl {
        cb += 16;
    }
    let x = col + 1;
    let y = row + 1;
    if mode.contains(TermMode::SGR_MOUSE) {
        let final_byte = if kind == MouseEventKind::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{cb};{x};{y}{final_byte}").into_bytes());
    }
    // Legacy X10 encoding. UTF-8 coordinates are supported with mode 1005.
    let utf8 = mode.contains(TermMode::UTF8_MOUSE);
    if !utf8 {
        if x > 223 || y > 223 {
            return None;
        }
        return Some(vec![
            0x1b,
            b'[',
            b'M',
            (32 + cb) as u8,
            (32 + x) as u8,
            (32 + y) as u8,
        ]);
    }
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(b"\x1b[M");
    out.push((32 + cb) as u8);
    let mut buf = [0u8; 4];
    out.extend_from_slice(char::from_u32(32 + x as u32).unwrap_or('?').encode_utf8(&mut buf).as_bytes());
    let mut buf2 = [0u8; 4];
    out.extend_from_slice(char::from_u32(32 + y as u32).unwrap_or('?').encode_utf8(&mut buf2).as_bytes());
    Some(out)
}

/// Encode focus-in/focus-out if the application requested focus reporting.
pub fn encode_focus(mode: TermMode, focused: bool) -> Option<Vec<u8>> {
    mode.contains(TermMode::FOCUS_IN_OUT).then(|| {
        if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        }
    })
}

/// Wrap pasted text according to bracketed-paste mode.
///
/// SECURITY: bracketed paste exists so an application can distinguish real
/// keystrokes from pasted text. That guarantee is void if the pasted text can
/// itself contain the `ESC [ 201 ~` end marker (or arbitrary escape sequences
/// that only work because the application trusts the paste body). C0 control
/// characters are therefore stripped from the paste body while keeping the
/// layout characters tab/newline/CR. This mirrors the filtering done by kitty
/// and alacritty; without it, text copied from a web page or a file can execute
/// terminal control sequences when pasted.
pub fn encode_paste(mode: TermMode, text: &str) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let mut out = Vec::with_capacity(text.len() + 12);
        out.extend_from_slice(b"\x1b[200~");
        for c in text.chars() {
            if c.is_control() {
                match c {
                    '\t' => out.push(b'\t'),
                    '\n' => out.push(b'\n'),
                    '\r' => out.push(b'\r'),
                    _ => {}
                }
            } else {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kitty_mode(flags: u32) -> TermMode {
        TermMode::KITTY_KEYBOARD_PROTOCOL & TermMode::from_bits_truncate(flags << 18)
    }

    #[test]
    fn plain_text_uses_utf8() {
        let mut m = TermMode::default();
        m.remove(TermMode::DISAMBIGUATE_ESC_CODES);
        let ev = KeyInput { modifiers: Modifiers { shift: true, ..Modifiers::NONE }, ..KeyInput::char('A') };
        assert_eq!(encode_key(m, &ev), b"A");
    }

    #[test]
    fn ctrl_c_is_etx() {
        let ev = KeyInput {
            modifiers: Modifiers { ctrl: true, ..Modifiers::NONE },
            text: Some("c".into()),
            ..KeyInput::char('c')
        };
        assert_eq!(encode_key(TermMode::default(), &ev), vec![3]);
    }

    #[test]
    fn ctrl_u_kitty() {
        let mode = kitty_mode(0b00001);
        let ev = KeyInput {
            modifiers: Modifiers { ctrl: true, ..Modifiers::NONE },
            text: Some("u".into()),
            ..KeyInput::char('u')
        };
        assert_eq!(encode_key(mode, &ev), b"\x1b[117;5u");
    }

    #[test]
    fn kitty_event_type_and_text() {
        let mode = kitty_mode(0b10011); // disambiguate + event types + text
        let ev = KeyInput {
            modifiers: Modifiers::NONE,
            text: Some("a".into()),
            kind: KeyEventKind::Press,
            ..KeyInput::char('a')
        };
        let out = encode_key(mode, &ev);
        assert_eq!(out, b"\x1b[97;1:1;97u");
    }

    #[test]
    fn kitty_disambiguate_keeps_enter_tab_backspace_legacy() {
        // kitty's disambiguate flag explicitly exempts Enter/Tab/Backspace so
        // that `reset` remains typeable after a crashed application. Before
        // the fix these were emitted as `CSI 13;1u` etc.
        let disambiguate = kitty_mode(0b1);
        for (named, expected) in [
            (NamedKey::Enter, b"\r".to_vec()),
            (NamedKey::Tab, b"\t".to_vec()),
            (NamedKey::Backspace, vec![0x7f]),
        ] {
            let ev = KeyInput { text: None, ..KeyInput::named(named) };
            assert_eq!(encode_key(disambiguate, &ev), expected, "key {named:?}");
        }
        // They must not produce release events without report-all-keys.
        let ev = KeyInput {
            kind: KeyEventKind::Release,
            text: None,
            ..KeyInput::named(NamedKey::Enter)
        };
        assert!(encode_key(kitty_mode(0b11), &ev).is_empty());

        // With report-all-keys they are ordinary CSI-u keys.
        let all = kitty_mode(0b1000);
        assert_eq!(encode_key(all, &KeyInput::named(NamedKey::Enter)), b"\x1b[13;1u");
        assert_eq!(encode_key(all, &KeyInput::named(NamedKey::Tab)), b"\x1b[9;1u");
        assert_eq!(
            encode_key(all, &KeyInput::named(NamedKey::Backspace)),
            b"\x1b[127;1u"
        );
    }

    #[test]
    fn kitty_event_types_alone_does_not_turn_text_into_csi_u() {
        // Report-event-types (flag 2) is not report-all-keys (flag 8): text
        // keys stay UTF-8 and have no release event, otherwise a program that
        // only asked for event types receives escape codes it cannot map back
        // to typed text.
        let event_types = kitty_mode(0b10);
        let press = KeyInput { text: Some("a".into()), ..KeyInput::char('a') };
        assert_eq!(encode_key(event_types, &press), b"a");
        let release = KeyInput {
            kind: KeyEventKind::Release,
            text: Some("a".into()),
            ..KeyInput::char('a')
        };
        assert!(encode_key(event_types, &release).is_empty());
        // Non-text keys still carry the event type.
        let up = KeyInput::named(NamedKey::Up);
        assert_eq!(encode_key(event_types, &up), b"\x1b[57352;1:1u");
    }

    #[test]
    fn kitty_shifted_keys_report_the_unshifted_codepoint() {
        // kitty: "the codepoint used is always the lower-case (or more
        // technically, un-shifted) version of the key". Shift+A used to be
        // encoded with key code 65 instead of 97.
        let mode = kitty_mode(0b1101); // disambiguate + alternates + all keys
        let ev = KeyInput {
            text: Some("A".into()),
            base_layout: Some('a'),
            modifiers: Modifiers { shift: true, ..Modifiers::NONE },
            ..KeyInput::char('A')
        };
        assert_eq!(encode_key(mode, &ev), b"\x1b[97:65;2u");

        // The same correction must work without a base-layout key for ASCII.
        let ev = KeyInput {
            text: Some("A".into()),
            modifiers: Modifiers { shift: true, ..Modifiers::NONE },
            ..KeyInput::char('A')
        };
        assert_eq!(encode_key(mode, &ev), b"\x1b[97:65;2u");

        // Associated text/event types keep working with the corrected code.
        let mode = kitty_mode(0b11111);
        let ev = KeyInput {
            text: Some("A".into()),
            base_layout: Some('a'),
            modifiers: Modifiers { shift: true, ..Modifiers::NONE },
            ..KeyInput::char('A')
        };
        assert_eq!(encode_key(mode, &ev), b"\x1b[97:65;2:1;65u");

        // Shifted punctuation maps back to the unshifted key as well.
        let ev = KeyInput {
            text: Some("!".into()),
            base_layout: Some('1'),
            modifiers: Modifiers { shift: true, ..Modifiers::NONE },
            ..KeyInput::char('!')
        };
        assert_eq!(encode_key(mode, &ev), b"\x1b[49:33;2:1;33u");
    }

    #[test]
    fn legacy_c0_ctrl_and_alt_combinations() {
        let ctrl_bs = KeyInput {
            modifiers: Modifiers { ctrl: true, ..Modifiers::NONE },
            ..KeyInput::named(NamedKey::Backspace)
        };
        assert_eq!(encode_key(TermMode::default(), &ctrl_bs), vec![0x08]);
        let alt_bs = KeyInput {
            modifiers: Modifiers { alt: true, ..Modifiers::NONE },
            ..KeyInput::named(NamedKey::Backspace)
        };
        assert_eq!(encode_key(TermMode::default(), &alt_bs), b"\x1b\x7f");
        let alt_tab = KeyInput {
            modifiers: Modifiers { alt: true, ..Modifiers::NONE },
            ..KeyInput::named(NamedKey::Tab)
        };
        assert_eq!(encode_key(TermMode::default(), &alt_tab), b"\x1b\t");
        let shift_alt_tab = KeyInput {
            modifiers: Modifiers { shift: true, alt: true, ..Modifiers::NONE },
            ..KeyInput::named(NamedKey::Tab)
        };
        assert_eq!(
            encode_key(TermMode::default(), &shift_alt_tab),
            b"\x1b\x1b[Z"
        );
    }

    #[test]
    fn arrows_legacy_and_kitty() {
        let m = TermMode::default();
        let ev = KeyInput::named(NamedKey::Up);
        assert_eq!(encode_key(m, &ev), b"\x1b[A");
        let mut app = m;
        app.insert(TermMode::APP_CURSOR);
        assert_eq!(encode_key(app, &ev), b"\x1bOA");
        let ev = KeyInput {
            modifiers: Modifiers { ctrl: true, ..Modifiers::NONE },
            ..KeyInput::named(NamedKey::Up)
        };
        assert_eq!(encode_key(m, &ev), b"\x1b[1;5A");
    }

    #[test]
    fn mouse_sgr_legacy_motion() {
        let mut m = TermMode::default();
        m.insert(TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE);
        let out = encode_mouse(m, MouseButton::Left, MouseEventKind::Press, 4, 2, Modifiers::NONE)
            .unwrap();
        assert_eq!(out, b"\x1b[<0;5;3M");
        assert!(encode_mouse(m, MouseButton::Left, MouseEventKind::Motion, 4, 2, Modifiers::NONE).is_none());
        allow_drag(&mut m);
        assert!(encode_mouse(m, MouseButton::Left, MouseEventKind::Motion, 4, 2, Modifiers::NONE).is_some());
        assert_eq!(
            encode_mouse(m, MouseButton::Left, MouseEventKind::Release, 4, 2, Modifiers::NONE).unwrap(),
            b"\x1b[<0;5;3m"
        );
    }

    fn allow_drag(m: &mut TermMode) {
        m.insert(TermMode::MOUSE_DRAG);
    }

    #[test]
    fn paste_wrapping() {
        assert_eq!(encode_paste(TermMode::default(), "x"), b"x");
        assert_eq!(
            encode_paste(TermMode::BRACKETED_PASTE, "x"),
            b"\x1b[200~x\x1b[201~"
        );
    }

    #[test]
    fn bracketed_paste_cannot_be_terminated_early() {
        // A hostile clipboard entry that contains the bracketed-paste end
        // marker (or any other escape sequence) must not escape the wrapper.
        let out = encode_paste(
            TermMode::BRACKETED_PASTE,
            "safe\x1b[201~\x1b[2Jevil\x1b]52;c;cHduZWQ=\x07",
        );
        assert!(out.starts_with(b"\x1b[200~"));
        assert!(out.ends_with(b"\x1b[201~"));
        let body = &out[6..out.len() - 6];
        assert!(!body.contains(&0x1b), "paste body must not contain ESC: {body:?}");
        assert_eq!(count_subslice(&out, b"\x1b[201~"), 1);
    }

    #[test]
    fn bracketed_paste_keeps_layout_controls() {
        let out = encode_paste(TermMode::BRACKETED_PASTE, "a\tb\nc\r\n");
        assert_eq!(out, b"\x1b[200~a\tb\nc\r\n\x1b[201~");
    }

    #[test]
    fn unbracketed_paste_is_unchanged_for_compatibility() {
        // Without bracketed paste the terminal has no framing contract to
        // protect; preserve the historical byte-for-byte behaviour.
        let text = "a\tb\nc";
        assert_eq!(encode_paste(TermMode::default(), text), text.as_bytes());
    }

    fn count_subslice(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|window| *window == needle)
            .count()
    }
}
