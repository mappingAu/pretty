//! Integration tests for the AUDIT-020 protocol/security fixes.
//!
//! These exercise the public `pretty-core` API end to end (PTY bytes ->
//! `StreamFilter` -> VTE parser / kitty graphics store -> `Frame` snapshot)
//! instead of the internal unit paths, so a regression in the wiring is caught
//! as well.

use base64::Engine as _;
use pretty_core::graphics::GraphicsStore;
use pretty_core::input::encode_paste;
use pretty_core::stream::StreamItem;
use pretty_core::{
    encode_paste as encode_paste_reexport, ClipboardKind, CoreConfig, TermMode, Terminal,
    TermSize, TerminalEvent,
};

fn recv_events(rx: &crossbeam_channel::Receiver<TerminalEvent>) -> Vec<TerminalEvent> {
    rx.try_iter().collect()
}

/// A 4 MiB+ OSC title sequence that the filter must discard without leaving the
/// VTE parser stuck inside the OSC string. Before the fix the parser received
/// `ESC ]` without a terminator and swallowed everything printed afterwards.
#[test]
fn oversized_osc_does_not_swallow_following_output() {
    let (mut term, _rx) = Terminal::new(TermSize::new(40, 5), &CoreConfig::default());
    let mut wire = b"\x1b]0;".to_vec();
    wire.extend(std::iter::repeat(b'x').take(4 * 1024 * 1024 + 64));
    wire.push(0x07);
    wire.extend_from_slice(b"hi");
    term.advance(&wire);
    let frame = term.snapshot();
    assert_eq!(frame.cell(0, 0).map(|c| c.c), Some('h'));
    assert_eq!(frame.cell(0, 1).map(|c| c.c), Some('i'));
}

/// Debug/testing view of a filtered stream, mirroring what `Terminal::advance`
/// receives. This keeps the assertion about the *wiring* (bytes vs graphics vs
/// notifications) inspectable from outside the crate.
#[test]
fn stream_filter_recovers_after_oversized_osc() {
    use pretty_core::stream::StreamFilter;
    let mut filter = StreamFilter::new();
    let mut wire = b"\x1b]0;".to_vec();
    wire.extend(std::iter::repeat(b'y').take(4 * 1024 * 1024 + 8));
    wire.push(0x07);
    wire.extend_from_slice(b"visible");
    let items = filter.feed(&wire);
    let bytes: Vec<u8> = items
        .iter()
        .flat_map(|item| match item {
            StreamItem::Bytes(bytes) => bytes.clone(),
            _ => Vec::new(),
        })
        .collect();
    assert!(
        bytes.starts_with(b"\x1b]\x07"),
        "OSC introducer must be closed for the VTE parser: {:?}",
        &bytes[..bytes.len().min(16)]
    );
    assert!(bytes.ends_with(b"visible"), "following output must pass through");
}

/// OSC 9/777 notification text is untrusted input: control characters and
/// bidi overrides must be stripped and bursts must be rate limited.
#[test]
fn notifications_are_sanitized_and_rate_limited() {
    let (mut term, rx) = Terminal::new(TermSize::new(40, 5), &CoreConfig::default());
    for _ in 0..32 {
        term.advance(b"\x1b]9;spam\x1b[31m\xe2\x80\xae\x07");
    }
    let notifications: Vec<(String, String)> = recv_events(&rx)
        .into_iter()
        .filter_map(|event| match event {
            TerminalEvent::Notification { title, body } => Some((title, body)),
            _ => None,
        })
        .collect();
    assert!(!notifications.is_empty(), "at least one notification should pass");
    assert!(
        notifications.len() <= 8,
        "notification burst must be rate limited, got {}",
        notifications.len()
    );
    for (title, body) in &notifications {
        assert!(!title.chars().any(char::is_control), "title: {title:?}");
        assert!(!body.chars().any(char::is_control), "body: {body:?}");
        assert!(!body.contains('\u{202e}'), "bidi override leaked: {body:?}");
        assert!(body.contains("spam"), "legitimate text should survive: {body:?}");
    }
}

/// A PNG whose IHDR declares a huge canvas must be rejected from the header,
/// before the decoder allocates pixel memory.
#[test]
fn png_decompression_bomb_is_rejected() {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&30_000u32.to_be_bytes());
    png.extend_from_slice(&30_000u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]);
    png.extend_from_slice(&0u32.to_be_bytes());
    png.extend_from_slice(b"malformed image data");
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);

    let mut store = GraphicsStore::new();
    let event = store.handle(format!("a=t,f=100;{b64}").as_bytes(), 0, 0);
    let response = event.response.unwrap_or_default();
    assert!(response.contains("EINVAL"), "response: {response:?}");
    assert!(response.contains("megapixel"), "response: {response:?}");
    assert_eq!(store.image_count(), 0);

    // A regular small PNG still works.
    let valid = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAFklEQVR4nGOsWvT9DAMewIRPcvgoAAD3iwLvLo6ZVwAAAABJRU5ErkJggg==";
    let event = store.handle(format!("a=t,f=100,i=1;{valid}").as_bytes(), 0, 0);
    assert!(event.response.unwrap_or_default().contains("OK"));
    assert_eq!(store.image_count(), 1);
}

/// Bracketed paste must not be terminable from the pasted payload.
#[test]
fn bracketed_paste_cannot_inject_escape_sequences() {
    let mut mode = TermMode::default();
    mode.insert(TermMode::BRACKETED_PASTE);
    let out = encode_paste(mode, "abc\x1b[201~\x1b]52;c;ZXZpbA==\x07def");
    assert!(out.starts_with(b"\x1b[200~"));
    assert!(out.ends_with(b"\x1b[201~"));
    let body = &out[6..out.len() - 6];
    assert!(
        !body.contains(&0x1b),
        "paste body must be escape-free: {body:?}"
    );
    // The re-export in the crate root is the function the app uses.
    assert_eq!(encode_paste_reexport(mode, "x"), b"\x1b[200~x\x1b[201~");
}

/// The kitty graphics anchor must follow content, not the current scroll
/// position: writing an image while the viewport is scrolled up used to bake
/// the scroll offset into the anchor and displace the image.
#[test]
fn kitty_image_anchor_is_scroll_independent() {
    let (mut term, _rx) = Terminal::new(TermSize::new(10, 5), &CoreConfig::default());
    term.advance(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\n");
    term.scroll(3);
    assert_eq!(term.display_offset(), 3);
    let raw = base64::engine::general_purpose::STANDARD.encode([10u8, 20, 30]);
    term.advance(format!("\x1b_Ga=T,f=24,s=1,v=1,i=9;{raw}\x1b\\").as_bytes());
    let frame = term.snapshot();
    assert_eq!(frame.images.len(), 1);
    assert_eq!(frame.images[0].anchor_line, 4, "bottom row grid line");
}

/// Verified-not-a-bug: UTF-8 code points split across PTY reads are reassembled
/// by the vte parser state machine (this is a regression guard for the stream
/// filter, which must not lose the partial lead bytes).
#[test]
fn utf8_split_across_pty_reads_is_reassembled() {
    let (mut term, _rx) = Terminal::new(TermSize::new(20, 5), &CoreConfig::default());
    term.advance(&[0xe4, 0xb8]); // first two bytes of U+4E2D
    term.advance(&[0xad]); // final byte
    let frame = term.snapshot();
    assert_eq!(frame.cell(0, 0).map(|c| c.c), Some('中'));
}

/// OSC 52 clipboard reads must be denied with the shipped default config.
#[test]
fn osc52_read_is_denied_by_default_config() {
    let config = CoreConfig::default();
    assert!(!config.osc52_paste, "secure default: clipboard reads denied");
    let (mut term, rx) = Terminal::new(TermSize::new(20, 5), &config);
    term.advance(b"\x1b]52;c;?\x07");
    let events = recv_events(&rx);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, TerminalEvent::ClipboardLoad(..))),
        "clipboard read reached the app: {events:?}"
    );

    // Writes (copy to clipboard) remain allowed.
    term.advance(b"\x1b]52;c;aGk=\x07");
    let events = recv_events(&rx);
    assert!(events.iter().any(|event| matches!(
        event,
        TerminalEvent::ClipboardStore(ClipboardKind::Clipboard, text) if text == "hi"
    )));
}

/// kitty: `q=2` means "suppress every response", including error responses.
#[test]
fn graphics_quiet_mode_suppresses_error_responses() {
    let mut store = GraphicsStore::new();
    assert!(store.handle(b"a=d,d=x,q=2", 0, 0).response.is_none());
    assert!(store.handle(b"a=d,d=x,q=1", 0, 0).response
        .unwrap_or_default()
        .contains("EINVAL"));
}
